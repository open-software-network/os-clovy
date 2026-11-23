import { formatClockTime, formatDate, intlLocale } from "../i18n/translate";

export const DATE_FORMAT_STORAGE_KEY = "clovy:date-format";
export const DATE_FORMAT_CHANGED_EVENT = "clovy:date-format-changed";

export type DateFormatPreference = "system" | "month-first" | "day-first";

export type DateFormatChangedDetail = {
  preference: DateFormatPreference;
};

export function getStoredDateFormat(): DateFormatPreference {
  if (typeof window === "undefined") return "system";
  try {
    return normalizeDateFormatPreference(window.localStorage.getItem(DATE_FORMAT_STORAGE_KEY));
  } catch {
    return "system";
  }
}

export function setStoredDateFormat(preference: DateFormatPreference) {
  try {
    window.localStorage.setItem(DATE_FORMAT_STORAGE_KEY, preference);
  } catch {
    // Locked-down WebViews may reject storage writes. Keep the live choice.
  }
  window.dispatchEvent(
    new CustomEvent<DateFormatChangedDetail>(DATE_FORMAT_CHANGED_EVENT, {
      detail: { preference },
    }),
  );
}

/**
 * A short calendar date ("Oct 15", "15 de out."). Month names always come from
 * the interface language; the preference only picks the day/month order.
 * `system` takes that order from the system's regional format only when the
 * system speaks the interface language (en-GB for an English UI); otherwise
 * it is the language's own pattern ("12 de out." on an en-US system).
 */
export function formatCalendarDate(
  date: Date,
  preference: DateFormatPreference,
  locales: Intl.LocalesArgument = intlLocale(),
  systemLocales: Intl.LocalesArgument = undefined,
) {
  const normalizedPreference = normalizeDateFormatPreference(preference);
  const formatter = new Intl.DateTimeFormat(locales, {
    month: "short",
    day: "numeric",
  });
  let order = normalizedPreference;
  if (order === "system") {
    if (language(systemLocales) !== language(locales)) return formatter.format(date);
    order = dayMonthOrder(systemLocales);
    // The language's own pattern keeps its connectors ("15 de out.") whenever
    // it already reads in the system's order.
    if (dayMonthOrder(locales) === order) return formatter.format(date);
  }

  // Format each component on its own so locale-specific units stay attached
  // when their order is forced (for example, Japanese `7月` and `9日`).
  const month = new Intl.DateTimeFormat(locales, { month: "short" }).format(date);
  const day = new Intl.DateTimeFormat(locales, { day: "numeric" }).format(date);
  return order === "month-first" ? `${month} ${day}` : `${day} ${month}`;
}

/** The language subtag a locale resolves to ("pt" for pt-BR). */
function language(locales: Intl.LocalesArgument): string {
  return new Intl.Locale(new Intl.DateTimeFormat(locales).resolvedOptions().locale).language;
}

/** Whether a locale writes the day or the month first in a short date. */
function dayMonthOrder(locales: Intl.LocalesArgument): "month-first" | "day-first" {
  const parts = new Intl.DateTimeFormat(locales, { month: "short", day: "numeric" })
    .formatToParts(new Date(2000, 6, 9))
    .map((part) => part.type);
  return parts.indexOf("day") < parts.indexOf("month") ? "day-first" : "month-first";
}

export function normalizeDateFormatPreference(value: unknown): DateFormatPreference {
  if (value === "month-first" || value === "day-first") return value;
  return "system";
}

/** Whether two dates fall on the same local calendar day. */
export function isSameLocalDay(left: Date, right: Date): boolean {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  );
}

const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * A list row's trailing timestamp: today the time ("2:09 PM"), within a week
 * the weekday ("Mon"), older the date ("May 22").
 */
export function formatListTimestamp(date: Date, now: Date = new Date()): string {
  if (isSameLocalDay(date, now)) {
    return formatClockTime(date);
  }
  if (Math.floor((now.getTime() - date.getTime()) / DAY_MS) < 7) {
    return formatDate(date, { weekday: "short" });
  }
  return formatDate(date, { month: "short", day: "numeric" });
}

export type RecentAge = { unit: "now" | "minutes" | "hours" | "days"; count: number };

/**
 * How long ago `date` was, in its largest whole unit, while under a week
 * (anything under a minute, or in the future, is "now"); null from a week on.
 */
export function recentAge(date: Date, nowMs: number = Date.now()): RecentAge | null {
  const diffMs = nowMs - date.getTime();
  if (diffMs < 60_000) return { unit: "now", count: 0 };
  const minutes = Math.floor(diffMs / 60_000);
  if (minutes < 60) return { unit: "minutes", count: minutes };
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return { unit: "hours", count: hours };
  const days = Math.floor(hours / 24);
  if (days < 7) return { unit: "days", count: days };
  return null;
}
