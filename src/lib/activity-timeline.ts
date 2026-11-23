import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const ACTIVITY_TIMELINE_EVENT = "clovy://activity-timeline";

export type ActivityTimelineAvailability =
  | "ready"
  | "neverEnabled"
  | "unsupported"
  | "keyMissing"
  | "error";

export type ActivityCategory =
  | "coding"
  | "codeReview"
  | "meeting"
  | "communication"
  | "design"
  | "documentation"
  | "planning"
  | "deploymentDevops"
  | "research"
  | "idlePersonal";

export type TimelineSessionDto = {
  id: number;
  appName: string;
  bundleId: string | null;
  contextKind: "domain" | "workspace" | null;
  context: string | null;
  startedAt: string;
  endedAt: string;
  durationMs: number;
  active: boolean;
  category: ActivityCategory;
  confidence: number;
  windowTitle: string | null;
};

export type TimelineGapKind = "idle" | "sleep" | "paused";
export type TimelinePauseReason = "manual" | "workHours" | "lowDisk" | "protectedVideo";

export type TimelineGapDto = {
  id: number;
  startedAt: string;
  endedAt: string;
  durationMs: number;
  kind: TimelineGapKind;
  pauseReason: TimelinePauseReason | null;
  /** Still going on: runs to the time of the read (`id` is 0). */
  ongoing: boolean;
};

export type TimelineStatsDto = {
  focusedMs: number;
  idleMs: number;
  awayMs: number;
  /** Apps told apart by bundle id; the name groups only sessions without one. */
  topApps: { appName: string; bundleId: string | null; durationMs: number }[];
  categories: { category: ActivityCategory; durationMs: number }[];
};

export type ActivityTimelineDto = {
  availability: ActivityTimelineAvailability;
  message: string | null;
  captureEnabled: boolean;
  sessions: TimelineSessionDto[];
  gaps: TimelineGapDto[];
  stats: TimelineStatsDto;
};

export type TimelineSessionDetailDto = {
  session: TimelineSessionDto;
  windows: {
    windowTitle: string | null;
    browserUrl: string | null;
    firstSeenAt: string;
    lastSeenAt: string;
    frameCount: number;
  }[];
  textExcerpt: string | null;
};

export type TimelineSearchResultDto = {
  sessionId: number;
  appName: string;
  windowTitle: string | null;
  browserUrl: string | null;
  seenAt: string;
  snippet: string;
};

export type ActivityTimelineSearchDto = {
  availability: ActivityTimelineAvailability;
  results: TimelineSearchResultDto[];
};

export async function activityTimeline(request: {
  from: string;
  to: string;
}): Promise<ActivityTimelineDto> {
  return invoke<ActivityTimelineDto>("activity_timeline", { request });
}

export async function activityTimelineSession(id: number): Promise<TimelineSessionDetailDto> {
  return invoke<TimelineSessionDetailDto>("activity_timeline_session", { request: { id } });
}

export async function activityTimelineSearch(request: {
  query: string;
  from?: string | null;
  to?: string | null;
  limit?: number | null;
}): Promise<ActivityTimelineSearchDto> {
  return invoke<ActivityTimelineSearchDto>("activity_timeline_search", {
    request: {
      query: request.query,
      from: request.from ?? null,
      to: request.to ?? null,
      limit: request.limit ?? null,
    },
  });
}

export async function onActivityTimelineUpdated(callback: () => void): Promise<UnlistenFn> {
  return listen(ACTIVITY_TIMELINE_EVENT, () => {
    callback();
  });
}

// One lookup per bundle id per session: icons do not change while Clovy runs.
const appIcons = new Map<string, Promise<string | null>>();

/**
 * The installed app's real icon (PNG data URL) resolved on this Mac by bundle
 * id, or null when the app is gone or its icon cannot be read.
 */
export function activityAppIcon(bundleId: string): Promise<string | null> {
  let icon = appIcons.get(bundleId);
  if (!icon) {
    icon = invoke<string | null>("activity_app_icon", { request: { bundleId } }).catch(() => null);
    appIcons.set(bundleId, icon);
  }
  return icon;
}

/**
 * Computes local day bounds (midnight to next local midnight) as ISO strings.
 * DST-safe via local Date constructor.
 */
export function dayRange(date: Date): { from: string; to: string } {
  const start = new Date(date.getFullYear(), date.getMonth(), date.getDate(), 0, 0, 0, 0);
  const end = new Date(date.getFullYear(), date.getMonth(), date.getDate() + 1, 0, 0, 0, 0);
  return {
    from: start.toISOString(),
    to: end.toISOString(),
  };
}
