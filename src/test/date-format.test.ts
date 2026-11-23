import { beforeEach, describe, expect, it } from "vitest";
import {
  DATE_FORMAT_STORAGE_KEY,
  type DateFormatPreference,
  formatCalendarDate,
  formatListTimestamp,
  getStoredDateFormat,
  isSameLocalDay,
  recentAge,
  setStoredDateFormat,
} from "../lib/date-format";

describe("date format preference", () => {
  beforeEach(() => localStorage.clear());

  it("defaults invalid stored values to the system format", () => {
    localStorage.setItem(DATE_FORMAT_STORAGE_KEY, "unknown");
    expect(getStoredDateFormat()).toBe("system");
  });

  it("persists a date order preference", () => {
    setStoredDateFormat("day-first");
    expect(getStoredDateFormat()).toBe("day-first");
  });

  it("formats month-first and day-first dates without changing the locale", () => {
    const date = new Date("2026-07-09T12:00:00Z");
    expect(formatCalendarDate(date, "month-first", "en-US")).toBe("Jul 9");
    expect(formatCalendarDate(date, "day-first", "en-US")).toBe("9 Jul");
    expect(formatCalendarDate(date, "month-first", "ja-JP")).toBe("7月 9日");
    expect(formatCalendarDate(date, "day-first", "ja-JP")).toBe("9日 7月");
  });

  it("falls back to the system format for an invalid runtime preference", () => {
    const date = new Date("2026-07-09T12:00:00Z");
    expect(formatCalendarDate(date, "unknown" as DateFormatPreference, "en-US")).toBe("Jul 9");
  });
});

describe("shared list dates", () => {
  const now = new Date(2026, 6, 15, 14, 30);

  it("compares local calendar days", () => {
    expect(isSameLocalDay(new Date(2026, 6, 15, 0, 1), now)).toBe(true);
    expect(isSameLocalDay(new Date(2026, 6, 14, 23, 59), now)).toBe(false);
    expect(isSameLocalDay(new Date(2025, 6, 15, 14, 30), now)).toBe(false);
  });

  it("shows the time today, the weekday within a week and the date after", () => {
    expect(formatListTimestamp(new Date(2026, 6, 15, 9, 5), now)).toBe(
      new Date(2026, 6, 15, 9, 5).toLocaleTimeString(undefined, {
        hour: "numeric",
        minute: "2-digit",
      }),
    );
    expect(formatListTimestamp(new Date(2026, 6, 12, 9, 5), now)).toBe(
      new Date(2026, 6, 12).toLocaleDateString(undefined, { weekday: "short" }),
    );
    expect(formatListTimestamp(new Date(2026, 6, 1, 9, 5), now)).toBe(
      new Date(2026, 6, 1).toLocaleDateString(undefined, { month: "short", day: "numeric" }),
    );
  });

  it("buckets a recent age into its largest whole unit under a week", () => {
    const at = (msAgo: number) => recentAge(new Date(now.getTime() - msAgo), now.getTime());
    expect(at(-5_000)).toEqual({ unit: "now", count: 0 });
    expect(at(59_000)).toEqual({ unit: "now", count: 0 });
    expect(at(5 * 60_000)).toEqual({ unit: "minutes", count: 5 });
    expect(at(3 * 3_600_000)).toEqual({ unit: "hours", count: 3 });
    expect(at(6 * 86_400_000)).toEqual({ unit: "days", count: 6 });
    expect(at(7 * 86_400_000)).toBeNull();
  });
});
