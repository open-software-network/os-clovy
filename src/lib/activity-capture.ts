import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { type CodingAgentSourcesDto, DEFAULT_CODING_AGENT_SOURCES } from "./coding-agents";

export const ACTIVITY_STATE_EVENT = "clovy://activity-state";

export type ActivityWorkHoursDto = {
  enabled: boolean;
  days: number[]; // ISO weekday 1=Mon..7=Sun
  start: string; // "HH:MM" 24h local
  end: string; // "HH:MM"
};

/** Activity notifications: the switch and quiet hours ("HH:MM", local). */
export type ActivityNotificationSettingsDto = {
  enabled: boolean;
  quietHours: { enabled: boolean; start: string; end: string };
};

export type ActivitySettingsDto = {
  enabled: boolean;
  secondaryMonitors: boolean;
  inputEvents: boolean;
  pauseOnProtectedVideo: boolean;
  ignoredApps: string[];
  ignoredDomains: string[];
  workHours: ActivityWorkHoursDto;
  retentionDays: number;
  /** When the day summary is generated on its own ("HH:MM", local). */
  daySummary: { time: string };
  notifications: ActivityNotificationSettingsDto;
  codingAgents: CodingAgentSourcesDto;
};

export type ActivityPauseReason = "manual" | "workHours" | "lowDisk" | "protectedVideo";

export type ActivityRequiredPermission = "accessibility" | "screenRecording";

export type ActivityStateDto =
  | { kind: "off" }
  | { kind: "active" }
  | { kind: "paused"; reason: ActivityPauseReason }
  | { kind: "needsPermissions"; missing: ActivityRequiredPermission[] }
  | { kind: "keyMissing" }
  | { kind: "error"; message: string };

export type ActivityPermissionState = "granted" | "denied" | "notDetermined" | "unsupported";

export type ActivityPermissionsDto = {
  accessibility: ActivityPermissionState;
  screenRecording: ActivityPermissionState;
  inputMonitoring: ActivityPermissionState;
};

export type ActivityStatusDto = {
  supported: boolean;
  settings: ActivitySettingsDto;
  state: ActivityStateDto;
  permissions: ActivityPermissionsDto;
  manualPause: boolean;
  lastFrameAt: string | null;
  debugExportAvailable: boolean;
};

export type ActivityDebugExportDto = {
  path: string;
  frames: number;
  secondaryFrames: number;
  inputEvents: number;
  pauses: number;
  codingAgentBlocks: number;
};

export const DEFAULT_ACTIVITY_SETTINGS: ActivitySettingsDto = {
  enabled: true,
  secondaryMonitors: false,
  inputEvents: true,
  pauseOnProtectedVideo: true,
  ignoredApps: [],
  ignoredDomains: [],
  workHours: {
    enabled: false,
    days: [1, 2, 3, 4, 5],
    start: "09:00",
    end: "18:00",
  },
  retentionDays: 30,
  daySummary: { time: "18:00" },
  notifications: {
    enabled: true,
    quietHours: { enabled: false, start: "22:00", end: "08:00" },
  },
  codingAgents: DEFAULT_CODING_AGENT_SOURCES,
};

export async function activityStatus(): Promise<ActivityStatusDto> {
  return invoke<ActivityStatusDto>("activity_status");
}

export async function activitySaveSettings(
  settings: ActivitySettingsDto,
): Promise<ActivityStatusDto> {
  return invoke<ActivityStatusDto>("activity_save_settings", {
    request: { settings },
  });
}

export async function activitySetPaused(paused: boolean): Promise<ActivityStatusDto> {
  return invoke<ActivityStatusDto>("activity_set_paused", {
    request: { paused },
  });
}

export async function activityRequestPermission(
  permission: "accessibility" | "screenRecording" | "inputMonitoring",
): Promise<ActivityStatusDto> {
  return invoke<ActivityStatusDto>("activity_request_permission", {
    request: { permission },
  });
}

export async function activityRecreateDatabase(): Promise<ActivityStatusDto> {
  return invoke<ActivityStatusDto>("activity_recreate_database");
}

export async function activityDebugExport(): Promise<ActivityDebugExportDto> {
  return invoke<ActivityDebugExportDto>("activity_debug_export");
}

export async function onActivityState(
  callback: (status: ActivityStatusDto) => void,
): Promise<UnlistenFn> {
  return listen<ActivityStatusDto>(ACTIVITY_STATE_EVENT, (event) => {
    callback(event.payload);
  });
}

export function normalizeDomain(raw: string): string {
  let s = raw.trim().toLowerCase();
  if (!s) return "";
  if (!/^[a-z0-9+.-]+:\/\//i.test(s)) {
    s = `https://${s}`;
  }
  try {
    const url = new URL(s);
    let host = url.hostname;
    if (host.startsWith("www.")) {
      host = host.slice(4);
    }
    return host;
  } catch {
    s = s.replace(/^[a-z0-9+.-]+:\/\//i, "");
    s = s.split(/[/?#:]/)[0] ?? "";
    if (s.startsWith("www.")) {
      s = s.slice(4);
    }
    return s;
  }
}
