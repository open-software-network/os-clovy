import {
  formatClockTime,
  getInterfaceLocale,
  type InterfaceLocale,
  intlLocale,
  type MessageKey,
} from "../../i18n";
import type {
  ActivityCategory,
  TimelineGapKind,
  TimelinePauseReason,
} from "../../lib/activity-timeline";
export const CATEGORY_COLOR_VARS: Record<ActivityCategory, string> = {
  coding: "var(--activity-coding)",
  codeReview: "var(--activity-code-review)",
  meeting: "var(--activity-meeting)",
  communication: "var(--activity-communication)",
  design: "var(--activity-design)",
  documentation: "var(--activity-documentation)",
  planning: "var(--activity-planning)",
  deploymentDevops: "var(--activity-deployment-devops)",
  research: "var(--activity-research)",
  idlePersonal: "var(--activity-idle-personal)",
};

export const CATEGORY_LABEL_KEYS: Record<ActivityCategory, MessageKey> = {
  coding: "activity.category.coding",
  codeReview: "activity.category.codeReview",
  meeting: "activity.category.meeting",
  communication: "activity.category.communication",
  design: "activity.category.design",
  documentation: "activity.category.documentation",
  planning: "activity.category.planning",
  deploymentDevops: "activity.category.deploymentDevops",
  research: "activity.category.research",
  idlePersonal: "activity.category.idlePersonal",
};

export function gapLabelKey(
  kind: TimelineGapKind,
  pauseReason: TimelinePauseReason | null,
): MessageKey {
  if (kind === "idle") return "activity.gap.idle";
  if (kind === "sleep") return "activity.gap.sleep";
  switch (pauseReason) {
    case "workHours":
      return "activity.gap.pausedWorkHours";
    case "lowDisk":
      return "activity.gap.pausedLowDisk";
    case "protectedVideo":
      return "activity.gap.pausedProtectedVideo";
    default:
      return "activity.gap.pausedManual";
  }
}

/**
 * Formats duration in milliseconds into a concise localized string (e.g. "1h 24m", "45m", "1 h 24 min").
 */
export function formatDuration(
  durationMs: number,
  locale: InterfaceLocale = getInterfaceLocale(),
): string {
  const isPt = locale === "pt-BR";
  if (durationMs <= 0) {
    return isPt ? "0 min" : "0m";
  }
  if (durationMs < 60_000) {
    return isPt ? "< 1 min" : "< 1m";
  }
  const totalMins = Math.round(durationMs / 60_000);
  const hours = Math.floor(totalMins / 60);
  const mins = totalMins % 60;

  if (hours === 0) {
    return isPt ? `${mins} min` : `${mins}m`;
  }
  if (mins === 0) {
    return isPt ? `${hours} h` : `${hours}h`;
  }
  return isPt ? `${hours} h ${mins} min` : `${hours}h ${mins}m`;
}

/**
 * Formats an ISO timestamp to local clock time (e.g. "09:30" or "9:30 AM").
 */
export function formatTime(
  dateStr: string,
  locale: InterfaceLocale = getInterfaceLocale(),
): string {
  return formatClockTime(dateStr, locale);
}

/**
 * Formats a local date heading (e.g. "Wednesday, Oct 4" or "quarta-feira, 4 de out.").
 */
export function formatDayHeader(
  date: Date,
  locale: InterfaceLocale = getInterfaceLocale(),
): string {
  return new Intl.DateTimeFormat(intlLocale(locale), {
    weekday: "long",
    month: "short",
    day: "numeric",
  }).format(date);
}

const TICK_EVERY_HOURS = 3;
/** Small enough to land on every local hour, including +x:30/+x:45 zones. */
const HOUR_SCAN_STEP_MS = 15 * 60_000;

export type LocalHour = { ms: number; hour: number; key: string };

/**
 * Every real local hour start in `[dayStartMs, dayEndMs)`: 23, 24, or 25 of
 * them. `key` is the hour's local "YYYY-MM-DDTHH", the same key the hour
 * reports use; on a fall-back day the repeated hour has two starts with one
 * key. `timeZone` defaults to the system zone (tests pin one).
 */
export function localHourStarts(
  dayStartMs: number,
  dayEndMs: number,
  timeZone?: string,
): LocalHour[] {
  const format = new Intl.DateTimeFormat("en-US", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  });
  const hours: LocalHour[] = [];
  for (let ms = dayStartMs; ms < dayEndMs; ms += HOUR_SCAN_STEP_MS) {
    const fields: Record<string, string> = {};
    for (const part of format.formatToParts(new Date(ms))) fields[part.type] = part.value;
    if (Number(fields.minute) !== 0) continue;
    const hour = fields.hour.padStart(2, "0");
    hours.push({
      ms,
      hour: Number(hour),
      key: `${fields.year}-${fields.month}-${fields.day}T${hour}`,
    });
  }
  return hours;
}

/**
 * Ruler ticks at real local clock hours (every 3 h) of the day
 * `[dayStartMs, dayEndMs)`, positioned by timestamp. On DST days the day is
 * 23 or 25 hours long: a skipped hour has no tick and a repeated one gets a
 * single tick. `timeZone` defaults to the system zone (tests pin one).
 */
export function hourTicks(
  dayStartMs: number,
  dayEndMs: number,
  timeZone?: string,
): { ms: number; label: string }[] {
  const ticks: { ms: number; label: string }[] = [];
  for (const { ms, hour } of localHourStarts(dayStartMs, dayEndMs, timeZone)) {
    const label = `${hour.toString().padStart(2, "0")}:00`;
    if (hour % TICK_EVERY_HOURS !== 0 || ticks.some((t) => t.label === label)) continue;
    ticks.push({ ms, label });
  }
  ticks.push({ ms: dayEndMs, label: "24:00" });
  return ticks;
}
