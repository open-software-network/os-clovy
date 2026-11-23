import { describe, expect, it } from "vitest";
import {
  compactScheduleLabel,
  draftFromSchedule,
  humanizeSchedule,
  scheduleFromDraft,
} from "../lib/routine-schedule";

/** Mirrors the formatter's locale-aware clock rendering so assertions hold
 * regardless of the machine locale running the suite. */
function time(hour: number, minute: number) {
  return new Date(2000, 0, 1, hour, minute).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
}

describe("humanizeSchedule", () => {
  it("translates daily schedules", () => {
    expect(humanizeSchedule("0 8 * * *")).toBe(`Every day at ${time(8, 0)}`);
    expect(humanizeSchedule("30 17 * * *")).toBe(`Every day at ${time(17, 30)}`);
    expect(humanizeSchedule("0 9,17 * * *")).toBe(`Every day at ${time(9, 0)} and ${time(17, 0)}`);
  });

  it("translates minute and hour cadences", () => {
    expect(humanizeSchedule("* * * * *")).toBe("Every minute");
    expect(humanizeSchedule("*/15 * * * *")).toBe("Every 15 minutes");
    expect(humanizeSchedule("*/1 * * * *")).toBe("Every minute");
    expect(humanizeSchedule("0 * * * *")).toBe("Every hour");
    expect(humanizeSchedule("30 * * * *")).toBe("Every hour at :30");
    expect(humanizeSchedule("0 */2 * * *")).toBe("Every 2 hours");
  });

  it("translates day-of-week schedules", () => {
    expect(humanizeSchedule("0 9 * * 1-5")).toBe(`Weekdays at ${time(9, 0)}`);
    expect(humanizeSchedule("0 10 * * 0,6")).toBe(`Weekends at ${time(10, 0)}`);
    expect(humanizeSchedule("0 8 * * 1")).toBe(`Every Monday at ${time(8, 0)}`);
    expect(humanizeSchedule("0 8 * * mon")).toBe(`Every Monday at ${time(8, 0)}`);
    // Cron allows 7 for Sunday alongside 0.
    expect(humanizeSchedule("0 8 * * 7")).toBe(`Every Sunday at ${time(8, 0)}`);
    expect(humanizeSchedule("0 8 * * 1,3,5")).toBe(
      `Every Monday, Wednesday, and Friday at ${time(8, 0)}`,
    );
    expect(humanizeSchedule("0 8 * * 1-3")).toBe(`Every Monday to Wednesday at ${time(8, 0)}`);
    expect(humanizeSchedule("0 8 * * 0-6")).toBe(`Every day at ${time(8, 0)}`);
  });

  it("translates monthly and yearly schedules", () => {
    expect(humanizeSchedule("0 9 1 * *")).toBe(`Monthly on the 1st at ${time(9, 0)}`);
    expect(humanizeSchedule("0 9 1,15 * *")).toBe(`Monthly on the 1st and 15th at ${time(9, 0)}`);
    expect(humanizeSchedule("0 9 22 * *")).toBe(`Monthly on the 22nd at ${time(9, 0)}`);
    expect(humanizeSchedule("0 9 11 6 *")).toBe(`Every year on Jun 11 at ${time(9, 0)}`);
    expect(humanizeSchedule("0 9 11 jun *")).toBe(`Every year on Jun 11 at ${time(9, 0)}`);
  });

  it("translates macros", () => {
    expect(humanizeSchedule("@hourly")).toBe("Every hour");
    expect(humanizeSchedule("@daily")).toBe(`Every day at ${time(0, 0)}`);
    expect(humanizeSchedule("@weekly")).toBe(`Every Sunday at ${time(0, 0)}`);
    expect(humanizeSchedule("@monthly")).toBe(`Monthly on the 1st at ${time(0, 0)}`);
    expect(humanizeSchedule("@yearly")).toBe(`Every year on Jan 1 at ${time(0, 0)}`);
  });

  it("accepts a cron: prefix", () => {
    expect(humanizeSchedule("cron: 0 8 * * *")).toBe(`Every day at ${time(8, 0)}`);
  });

  it("phrases the stored interval grammar", () => {
    expect(humanizeSchedule("every 30m")).toBe("Every 30 minutes");
    expect(humanizeSchedule("every 1m")).toBe("Every minute");
    expect(humanizeSchedule("every 1h")).toBe("Every hour");
    expect(humanizeSchedule("every 2h")).toBe("Every 2 hours");
    expect(humanizeSchedule("every 90 minutes")).toBe("Every 90 minutes");
  });

  it("passes non-cron schedules through unchanged", () => {
    expect(humanizeSchedule("every day at 9:00")).toBe("every day at 9:00");
    expect(humanizeSchedule("2026-06-12T09:00:00")).toBe("2026-06-12T09:00:00");
    expect(humanizeSchedule("")).toBe("");
  });

  it("passes invalid or exotic cron expressions through unchanged", () => {
    // Out-of-range minute.
    expect(humanizeSchedule("61 9 * * *")).toBe("61 9 * * *");
    // Six fields (seconds-style cron) is not the gateway's format.
    expect(humanizeSchedule("0 0 9 * * *")).toBe("0 0 9 * * *");
    // Stepped range: too exotic to phrase confidently.
    expect(humanizeSchedule("1-5/2 * * * *")).toBe("1-5/2 * * * *");
    // Day-of-month and day-of-week combined use cron's OR semantics; saying
    // it wrongly is worse than showing the expression.
    expect(humanizeSchedule("0 9 1 * 1")).toBe("0 9 1 * 1");
    expect(humanizeSchedule("*/0 * * * *")).toBe("*/0 * * * *");
  });
});

describe("compactScheduleLabel", () => {
  it("removes redundant at before clock times", () => {
    expect(compactScheduleLabel("0 9 * * 1-5")).toBe(`Weekdays ${time(9, 0)}`);
    expect(compactScheduleLabel("0 8 * * 1")).toBe(`Every Monday ${time(8, 0)}`);
  });

  it("leaves non-clock at phrases unchanged", () => {
    expect(compactScheduleLabel("30 * * * *")).toBe("Every hour at :30");
  });
});

describe("scheduleFromDraft", () => {
  it("renders day-based drafts as five-field cron", () => {
    expect(scheduleFromDraft({ kind: "daily", time: "09:00" })).toBe("0 9 * * *");
    expect(scheduleFromDraft({ kind: "weekdays", time: "08:30" })).toBe("30 8 * * 1-5");
    expect(scheduleFromDraft({ kind: "weekly", day: 5, time: "16:00" })).toBe("0 16 * * 5");
  });

  it("renders intervals in the persisted grammar, preferring hours", () => {
    expect(scheduleFromDraft({ kind: "interval", minutes: 30 })).toBe("every 30m");
    expect(scheduleFromDraft({ kind: "interval", minutes: 120 })).toBe("every 2h");
  });

  it("passes custom expressions through trimmed", () => {
    expect(scheduleFromDraft({ kind: "custom", expression: " 0 9 1 * * " })).toBe("0 9 1 * *");
  });
});

describe("draftFromSchedule", () => {
  it("maps the presets back onto the picker", () => {
    expect(draftFromSchedule("0 9 * * *")).toEqual({
      kind: "daily",
      time: "09:00",
    });
    expect(draftFromSchedule("30 8 * * 1-5")).toEqual({
      kind: "weekdays",
      time: "08:30",
    });
    expect(draftFromSchedule("0 16 * * 5")).toEqual({
      kind: "weekly",
      day: 5,
      time: "16:00",
    });
    // Imported intervals may be normalized to minutes; an hour's worth still maps.
    expect(draftFromSchedule("every 90m")).toEqual({
      kind: "interval",
      minutes: 90,
    });
    expect(draftFromSchedule("every 2h")).toEqual({
      kind: "interval",
      minutes: 120,
    });
  });

  it("parks anything it cannot phrase in custom, unchanged", () => {
    expect(draftFromSchedule("*/15 * * * *")).toEqual({
      kind: "custom",
      expression: "*/15 * * * *",
    });
    expect(draftFromSchedule("0 9 1 * *")).toEqual({
      kind: "custom",
      expression: "0 9 1 * *",
    });
    expect(draftFromSchedule("2026-07-01T09:00")).toEqual({
      kind: "custom",
      expression: "2026-07-01T09:00",
    });
  });

  it("round-trips every preset draft", () => {
    const drafts = [
      { kind: "daily", time: "07:05" },
      { kind: "weekdays", time: "18:45" },
      { kind: "weekly", day: 0, time: "10:00" },
      { kind: "interval", minutes: 45 },
    ] as const;
    for (const draft of drafts) {
      expect(draftFromSchedule(scheduleFromDraft(draft))).toEqual(draft);
    }
  });
});
