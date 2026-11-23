/** Turns stored routine schedules into plain language
 * for the Routines list ("0 8 * * 1-5" reads as "Weekdays at 8:00 AM").
 *
 * Imported schedules are preserved verbatim, so the field may hold a
 * five-field cron expression, an interval ("every 30m"), a one-off ISO date,
 * or text that is already human. Cron expressions and the stored interval
 * grammar are phrased in the interface language; every other input — and any
 * cron shape too exotic to phrase with confidence (combined day-of-month +
 * day-of-week restrictions, stepped ranges) — is returned unchanged rather
 * than risk describing a schedule wrongly. */

import { getInterfaceLocale } from "../i18n/locale";
import { formatDate, formatList, formatClockTime as formatTime, t } from "../i18n/translate";

type CronField =
  | { kind: "any" }
  | { kind: "step"; step: number }
  | { kind: "values"; values: number[] };

const DAY_NAMES = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const DAY_TOKENS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const MONTH_NAMES = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];
const MONTH_TOKENS = [
  "jan",
  "feb",
  "mar",
  "apr",
  "may",
  "jun",
  "jul",
  "aug",
  "sep",
  "oct",
  "nov",
  "dec",
];

const MACROS: Record<string, string> = {
  "@hourly": "0 * * * *",
  "@daily": "0 0 * * *",
  "@midnight": "0 0 * * *",
  "@weekly": "0 0 * * 0",
  "@monthly": "0 0 1 * *",
  "@yearly": "0 0 1 1 *",
  "@annually": "0 0 1 1 *",
};

/** The stored interval grammar ("every 30m", "every 2h"), as minutes. The
 * stored text is a wire value and stays English; only its display is
 * translated. */
const INTERVAL_PATTERN = /^every\s+(\d+)\s*(m|h|min|minutes?|hours?)$/i;

function intervalMinutes(schedule: string): number | null {
  const interval = schedule.match(INTERVAL_PATTERN);
  if (!interval) return null;
  const amount = Number(interval[1]);
  const minutes = interval[2].toLowerCase().startsWith("h") ? amount * 60 : amount;
  return minutes > 0 ? minutes : null;
}

function intervalPhrase(minutes: number): string {
  if (minutes % 60 === 0) {
    const hours = minutes / 60;
    return hours === 1
      ? t("lib.schedule.everyHour")
      : t("lib.schedule.everyNHours", { count: hours });
  }
  return minutes === 1
    ? t("lib.schedule.everyMinute")
    : t("lib.schedule.everyNMinutes", { count: minutes });
}

export function humanizeSchedule(schedule: string): string {
  const trimmed = schedule.trim();
  const interval = intervalMinutes(trimmed);
  if (interval !== null) return intervalPhrase(interval);
  const source = MACROS[trimmed.toLowerCase()] ?? trimmed.replace(/^cron:?\s+/i, "");
  const fields = source.split(/\s+/);
  if (fields.length !== 5) return schedule;

  const minute = parseField(fields[0], 0, 59);
  const hour = parseField(fields[1], 0, 23);
  const dayOfMonth = parseField(fields[2], 1, 31);
  const month = parseField(fields[3], 1, 12, MONTH_TOKENS);
  const dayOfWeek = parseField(fields[4], 0, 7, DAY_TOKENS);
  if (!minute || !hour || !dayOfMonth || !month || !dayOfWeek) return schedule;

  return phrase(minute, hour, dayOfMonth, month, normalizeDayOfWeek(dayOfWeek)) ?? schedule;
}

export function compactScheduleLabel(schedule: string): string {
  const label = humanizeSchedule(schedule);
  if (!isEnglish()) return label.replace(/(^|\s)às (?=\d{1,2}:\d{2}\b)/, "$1");
  return label.replace(/\bat (?=\d{1,2}:\d{2}\b)/, "");
}

function parseField(raw: string, min: number, max: number, names?: string[]): CronField | null {
  if (raw === "*") return { kind: "any" };

  const step = raw.match(/^\*\/(\d+)$/);
  if (step) {
    const value = Number(step[1]);
    if (value < 1) return null;
    return value === 1 ? { kind: "any" } : { kind: "step", step: value };
  }

  const values: number[] = [];
  for (const part of raw.split(",")) {
    const range = part.match(/^([a-z0-9]+)-([a-z0-9]+)$/i);
    if (range) {
      const from = tokenValue(range[1], min, max, names);
      const to = tokenValue(range[2], min, max, names);
      if (from === null || to === null || to < from) return null;
      for (let value = from; value <= to; value += 1) values.push(value);
      continue;
    }
    const value = tokenValue(part, min, max, names);
    if (value === null) return null;
    values.push(value);
  }
  if (values.length === 0) return null;
  return { kind: "values", values: [...new Set(values)].sort((a, b) => a - b) };
}

function tokenValue(token: string, min: number, max: number, names?: string[]): number | null {
  if (names) {
    const index = names.indexOf(token.slice(0, 3).toLowerCase());
    if (index !== -1) return index + Math.min(min, 1);
  }
  if (!/^\d+$/.test(token)) return null;
  const value = Number(token);
  return value >= min && value <= max ? value : null;
}

/** Cron accepts both 0 and 7 for Sunday; a full 0-6 list is just "any day". */
function normalizeDayOfWeek(field: CronField): CronField {
  if (field.kind !== "values") return field;
  const values = [...new Set(field.values.map((value) => (value === 7 ? 0 : value)))].sort(
    (a, b) => a - b,
  );
  if (values.length === 7) return { kind: "any" };
  return { kind: "values", values };
}

function phrase(
  minute: CronField,
  hour: CronField,
  dayOfMonth: CronField,
  month: CronField,
  dayOfWeek: CronField,
): string | null {
  const unrestrictedDate =
    dayOfMonth.kind === "any" && month.kind === "any" && dayOfWeek.kind === "any";

  if (unrestrictedDate) {
    if (minute.kind === "any" && hour.kind === "any") return t("lib.schedule.everyMinute");
    if (minute.kind === "step" && hour.kind === "any")
      return t("lib.schedule.everyNMinutes", { count: minute.step });
    const fixedMinute = singleValue(minute);
    if (fixedMinute !== null && hour.kind === "any")
      return fixedMinute === 0
        ? t("lib.schedule.everyHour")
        : t("lib.schedule.everyHourAtMinute", { minute: String(fixedMinute).padStart(2, "0") });
    if (fixedMinute === 0 && hour.kind === "step")
      return t("lib.schedule.everyNHours", { count: hour.step });
    const time = timeText(minute, hour);
    return time ? t("lib.schedule.everyDayAt", { time }) : null;
  }

  const time = timeText(minute, hour);
  if (!time) return null;

  if (dayOfWeek.kind === "values" && dayOfMonth.kind === "any" && month.kind === "any")
    return dayPhrase(dayOfWeek.values, time);

  if (dayOfMonth.kind === "values" && dayOfWeek.kind === "any" && month.kind === "any")
    return t("lib.schedule.monthlyAt", {
      count: dayOfMonth.values.length,
      days: joinAnd(dayOfMonth.values.map(ordinal)),
      time,
    });

  if (
    month.kind === "values" &&
    month.values.length === 1 &&
    dayOfMonth.kind === "values" &&
    dayOfWeek.kind === "any"
  ) {
    const monthIndex = month.values[0] - 1;
    const dates = dayOfMonth.values.map((day) => monthDayText(monthIndex, day));
    return t("lib.schedule.yearlyAt", { dates: joinAnd(dates), time });
  }

  return null;
}

function isEnglish(): boolean {
  return getInterfaceLocale() === "en";
}

/** "Jan 5" in English (unchanged), the locale's own short month-day
 * ("5 de jan.") elsewhere. */
function monthDayText(monthIndex: number, day: number): string {
  if (isEnglish()) return `${MONTH_NAMES[monthIndex]} ${day}`;
  return formatDate(new Date(2000, monthIndex, day), { month: "short", day: "numeric" });
}

/** Weekday name for a cron day (0 = Sunday). English keeps its fixed names;
 * other languages take the name from Intl ("segunda-feira"). */
export function dayName(day: number): string {
  if (isEnglish()) return DAY_NAMES[day];
  // 2023-01-01 was a Sunday, so day N of that week is weekday N.
  return formatDate(new Date(2023, 0, 1 + day), { weekday: "long" });
}

/** A concrete clock time needs one minute value and one or more hour values;
 * anything else (wildcards, steps) has no single time to print. */
function timeText(minute: CronField, hour: CronField): string | null {
  const fixedMinute = singleValue(minute);
  if (fixedMinute === null || hour.kind !== "values") return null;
  return joinAnd(hour.values.map((h) => formatClockTime(h, fixedMinute)));
}

function dayPhrase(days: number[], time: string): string {
  if (sameValues(days, [1, 2, 3, 4, 5])) return t("lib.schedule.weekdaysAt", { time });
  if (sameValues(days, [0, 6])) return t("lib.schedule.weekendsAt", { time });
  if (days.length === 1) {
    const day = dayName(days[0]);
    // Portuguese weekend day names are masculine ("Todo sábado"), the rest
    // feminine ("Toda segunda-feira"); English reads the same either way.
    return days[0] === 0 || days[0] === 6
      ? t("lib.schedule.everyWeekendDayAt", { day, time })
      : t("lib.schedule.everyWeekdayAt", { day, time });
  }
  const contiguous = days.every((value, index) => index === 0 || value === days[index - 1] + 1);
  if (contiguous && days.length >= 3)
    return t("lib.schedule.dayRangeAt", {
      from: dayName(days[0]),
      to: dayName(days[days.length - 1]),
      time,
    });
  return t("lib.schedule.daysAt", { days: joinAnd(days.map(dayName)), time });
}

function formatClockTime(hourOfDay: number, minute: number): string {
  return formatTime(new Date(2000, 0, 1, hourOfDay, minute));
}

function singleValue(field: CronField): number | null {
  return field.kind === "values" && field.values.length === 1 ? field.values[0] : null;
}

function sameValues(left: number[], right: number[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function joinAnd(parts: string[]): string {
  if (parts.length <= 1) return parts[0] ?? "";
  if (!isEnglish()) return formatList(parts, { type: "conjunction" });
  if (parts.length === 2) return `${parts[0]} and ${parts[1]}`;
  return `${parts.slice(0, -1).join(", ")}, and ${parts[parts.length - 1]}`;
}

/** The structured shape behind the routine editor's schedule picker. Covers
 * the schedules the picker can express; anything else round-trips through
 * `custom` untouched so an exotic cron expression survives an unrelated edit.
 * `time` is the "HH:MM" shape of `<input type="time">`. */
export type ScheduleDraft =
  | { kind: "daily"; time: string }
  | { kind: "weekdays"; time: string }
  | { kind: "weekly"; day: number; time: string }
  | { kind: "interval"; minutes: number }
  | { kind: "custom"; expression: string };

/** Renders a draft as a Clovy schedule string: five-field cron for the
 * clock-time kinds, the "every Nm"/"every Nh" interval grammar, or the
 * custom expression verbatim. */
export function scheduleFromDraft(draft: ScheduleDraft): string {
  switch (draft.kind) {
    case "daily":
      return `${cronTime(draft.time)} * * *`;
    case "weekdays":
      return `${cronTime(draft.time)} * * 1-5`;
    case "weekly":
      return `${cronTime(draft.time)} * * ${draft.day}`;
    case "interval":
      return draft.minutes % 60 === 0 ? `every ${draft.minutes / 60}h` : `every ${draft.minutes}m`;
    case "custom":
      return draft.expression.trim();
  }
}

function cronTime(time: string): string {
  const [hour, minute] = time.split(":").map(Number);
  return `${minute || 0} ${hour || 0}`;
}

/** Maps a stored schedule back onto the picker. Mirrors the subset
 * `scheduleFromDraft` emits — a fixed clock time daily, on weekdays, or on
 * one weekday, plus normalized intervals, and parks everything else
 * in `custom` rather than misreading it. */
export function draftFromSchedule(schedule: string): ScheduleDraft {
  const trimmed = schedule.trim();

  const minutes = intervalMinutes(trimmed);
  if (minutes !== null) return { kind: "interval", minutes };

  const fields = trimmed.split(/\s+/);
  if (fields.length === 5) {
    const minute = singleValue(parseField(fields[0], 0, 59) ?? { kind: "any" });
    const hour = singleValue(parseField(fields[1], 0, 23) ?? { kind: "any" });
    const dayOfWeek = parseField(fields[4], 0, 7, DAY_TOKENS);
    const dateFree = fields[2] === "*" && fields[3] === "*";
    if (minute !== null && hour !== null && dateFree && dayOfWeek) {
      const time = `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}`;
      const days = normalizeDayOfWeek(dayOfWeek);
      if (days.kind === "any") return { kind: "daily", time };
      if (days.kind === "values" && sameValues(days.values, [1, 2, 3, 4, 5]))
        return { kind: "weekdays", time };
      if (days.kind === "values" && days.values.length === 1)
        return { kind: "weekly", day: days.values[0], time };
    }
  }

  return { kind: "custom", expression: trimmed };
}

function ordinal(value: number): string {
  // Portuguese reads plain day numbers ("nos dias 1 e 15").
  if (!isEnglish()) return String(value);
  const tens = value % 100;
  if (tens >= 11 && tens <= 13) return `${value}th`;
  const suffix = { 1: "st", 2: "nd", 3: "rd" }[value % 10] ?? "th";
  return `${value}${suffix}`;
}
