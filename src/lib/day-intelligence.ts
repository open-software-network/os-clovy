import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ActivityCategory, ActivityTimelineAvailability } from "./activity-timeline";

/** Reports, workstreams, or a summary changed, or a run started or ended. */
export const DAY_INTELLIGENCE_EVENT = "clovy://day-intelligence";
/** An activity notification was clicked: open "Today" at `day`'s summary. */
export const TODAY_OPEN_EVENT = "clovy:today:open";

export type DayIntelligenceProviderState = "ready" | "missing" | "insufficient";
export type EmbedderStatus = "absent" | "downloading" | "ready" | "failed";

/** A measured stretch of work (UTC, RFC 3339). */
export type ActivityPeriodDto = { startedAt: string; endedAt: string };

export type HourActivityDto = {
  description: string;
  minutes: number;
  seconds?: number | null;
  /** Measured sessions the activity happened in; null when only the hour is known. */
  periods: ActivityPeriodDto[] | null;
};

export type HourReportDto = {
  hour: string; // local "YYYY-MM-DDTHH"
  startedAt: string;
  endedAt: string;
  activeMinutes: number;
  summary: string;
  activities: HourActivityDto[];
  provider: string;
  generatedAt: string;
};

export type WorkstreamHourDto = {
  hour: string; // local "YYYY-MM-DDTHH"
  minutes: number;
  seconds?: number;
  note: string;
  /**
   * Bounds of the measured sessions the hour's work was located in: where,
   * not how long (a session can hold several workstreams). Null for a record
   * known only by hour.
   */
  periods: ActivityPeriodDto[] | null;
  /** Of `minutes`, those located in `periods`; the rest is known only by hour. */
  locatedMinutes: number;
  locatedSeconds?: number;
};

export type WorkstreamDto = {
  id: number;
  title: string;
  summary: string;
  minutes: number;
  seconds?: number;
  hours: WorkstreamHourDto[];
};

export type StandupDto = { done: string[]; inProgress: string[]; blockers: string[] };

export type DaySummaryDto = {
  day: string;
  headline: string;
  narrative: string;
  insights: { title: string; text: string }[];
  standup: StandupDto;
  hoursCovered: number;
  locale: string;
  provider: string;
  trigger: "scheduled" | "manual";
  generatedAt: string;
};

export type DayPanelsDto = {
  focusedMs: number;
  idleMs: number;
  awayMs: number;
  categories: { category: ActivityCategory; durationMs: number }[];
  topApps: { appName: string; bundleId: string | null; durationMs: number }[];
  hours: { hour: string; focusedMs: number }[];
  workstreams: { id: number; title: string; minutes: number; seconds?: number }[];
  meetingCount: number;
  meetingMs: number;
  codingAgentBlocks: number;
  codingAgentActiveSeconds: number;
};

export type DayIntelligenceDto = {
  availability: ActivityTimelineAvailability;
  day: string;
  provider: DayIntelligenceProviderState;
  embedder: EmbedderStatus;
  running: boolean;
  summaryTime: string;
  summary: DaySummaryDto | null;
  workstreams: WorkstreamDto[];
  hourReports: HourReportDto[];
  panels: DayPanelsDto;
};

/** Local "YYYY-MM-DD" of a date. */
export function dayKey(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

export function dateOfDayKey(day: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!match) return null;
  return new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
}

export async function dayIntelligence(day: string): Promise<DayIntelligenceDto> {
  return invoke<DayIntelligenceDto>("day_intelligence_day", { request: { day } });
}

export async function generateDaySummary(day: string): Promise<DayIntelligenceDto> {
  return invoke<DayIntelligenceDto>("day_intelligence_generate", { request: { day } });
}

export async function onDayIntelligenceUpdated(
  callback: (payload: { day: string; running: boolean }) => void,
): Promise<UnlistenFn> {
  return listen<{ day: string; running: boolean }>(DAY_INTELLIGENCE_EVENT, (event) =>
    callback(event.payload),
  );
}

/**
 * Tells the backend the webview can receive "clovy:today:open" events and
 * returns the day of an activity notification clicked before that.
 */
export async function todayOpenReady(): Promise<string | null> {
  return invoke<string | null>("today_open_ready");
}

/** The standup as bullet lists under its three headings, ready to paste. */
export function formatStandup(
  standup: StandupDto,
  labels: { done: string; inProgress: string; blockers: string; none: string },
): string {
  const section = (title: string, lines: string[]) =>
    [`${title}:`, ...(lines.length > 0 ? lines : [labels.none]).map((line) => `- ${line}`)].join(
      "\n",
    );
  return [
    section(labels.done, standup.done),
    section(labels.inProgress, standup.inProgress),
    section(labels.blockers, standup.blockers),
  ].join("\n\n");
}

// A notification click can arrive before the "Today" view is mounted (it is
// lazy-loaded), so the requested day waits here until the view takes it.
let pendingTodayOpen: string | null = null;
const TODAY_OPEN_REQUEST = "clovy:today:open-request";

/** Asks the "Today" view to show `day`'s summary (mounted or not). */
export function requestTodayOpen(day: string) {
  pendingTodayOpen = day;
  window.dispatchEvent(new CustomEvent(TODAY_OPEN_REQUEST, { detail: { day } }));
}

/** The day a notification asked for, once. */
export function takePendingTodayOpen(): string | null {
  const day = pendingTodayOpen;
  pendingTodayOpen = null;
  return day;
}

export function onTodayOpenRequest(callback: (day: string) => void): () => void {
  const handler = (event: Event) => {
    const day = (event as CustomEvent<{ day: string }>).detail?.day;
    if (day) {
      pendingTodayOpen = null;
      callback(day);
    }
  };
  window.addEventListener(TODAY_OPEN_REQUEST, handler);
  return () => window.removeEventListener(TODAY_OPEN_REQUEST, handler);
}
