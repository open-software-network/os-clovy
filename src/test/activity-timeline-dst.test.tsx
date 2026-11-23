import { describe, expect, it } from "vitest";
import { hourTicks } from "../components/activity-timeline/timeline-utils";

// The Today strip positions a tick at `(tick.ms - dayStart) / (dayEnd - dayStart)`,
// the same scale as sessions and gaps. New York local days, given as their
// UTC instants (local midnight to next local midnight).
const ZONE = "America/New_York";

function fractions(fromIso: string, toIso: string) {
  const start = Date.parse(fromIso);
  const end = Date.parse(toIso);
  const byLabel: Record<string, number> = {};
  for (const tick of hourTicks(start, end, ZONE)) {
    byLabel[tick.label] = (tick.ms - start) / (end - start);
  }
  return { byLabel, hours: (end - start) / 3_600_000 };
}

describe("Today strip hour ruler", () => {
  it("places ticks at their real local time on a 23-hour spring-forward day", () => {
    const { byLabel, hours } = fractions("2026-03-08T05:00:00Z", "2026-03-09T04:00:00Z");
    expect(hours).toBe(23);
    expect(byLabel["00:00"]).toBe(0);
    // 03:00 comes two elapsed hours after midnight (02:00 does not exist).
    expect(byLabel["03:00"]).toBeCloseTo(2 / 23, 9);
    expect(byLabel["06:00"]).toBeCloseTo(5 / 23, 9);
    expect(byLabel["24:00"]).toBe(1);
  });

  it("places ticks at their real local time on a 25-hour fall-back day", () => {
    const { byLabel, hours } = fractions("2026-11-01T04:00:00Z", "2026-11-02T05:00:00Z");
    expect(hours).toBe(25);
    // 01:00 repeats, so 03:00 is four elapsed hours after midnight.
    expect(byLabel["03:00"]).toBeCloseTo(4 / 25, 9);
    expect(byLabel["12:00"]).toBeCloseTo(13 / 25, 9);
  });

  it("keeps the even 24-hour layout on an ordinary day", () => {
    const { byLabel } = fractions("2026-06-15T04:00:00Z", "2026-06-16T04:00:00Z");
    expect(Object.keys(byLabel)).toEqual([
      "00:00",
      "03:00",
      "06:00",
      "09:00",
      "12:00",
      "15:00",
      "18:00",
      "21:00",
      "24:00",
    ]);
    expect(byLabel["12:00"]).toBeCloseTo(0.5, 9);
  });
});
