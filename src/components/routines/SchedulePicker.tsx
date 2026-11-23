import {
  dayName,
  humanizeSchedule,
  scheduleFromDraft,
  type ScheduleDraft,
} from "../../lib/routine-schedule";
import { type TFunction, useT } from "../../i18n";
import { Select } from "../ui/Select";

function kindOptions(t: TFunction) {
  return [
    { value: "daily", label: t("routines.schedule.daily") },
    { value: "weekdays", label: t("routines.schedule.weekdays") },
    { value: "weekly", label: t("routines.schedule.weekly") },
    { value: "interval", label: t("routines.schedule.interval") },
    { value: "custom", label: t("routines.schedule.custom") },
  ];
}

type ScheduleKind = ScheduleDraft["kind"];

/** Weekday names in the interface language, Sunday first (cron day 0). English
 * keeps its fixed names regardless of the system's regional format. */
function dayOptions() {
  return Array.from({ length: 7 }, (_, day) => {
    const name = dayName(day);
    return { value: String(day), label: name.charAt(0).toUpperCase() + name.slice(1) };
  });
}

function unitOptions(t: TFunction) {
  return [
    { value: "minutes", label: t("routines.schedule.minutes") },
    { value: "hours", label: t("routines.schedule.hours") },
  ];
}

/** Structured schedule editing for routines: one row where the cadence
 * select swaps its companion controls inline. Presets cover the schedules
 * people actually set (a clock time daily, on weekdays, or weekly, plus
 * simple intervals); Custom accepts any preserved schedule expression and is where
 * un-presentable existing schedules land untouched. */
export function SchedulePicker({
  draft,
  onChange,
}: {
  draft: ScheduleDraft;
  onChange: (draft: ScheduleDraft) => void;
}) {
  const t = useT();
  // Carry the clock time across the day-based kinds so flipping
  // Daily → Weekdays keeps the chosen time.
  const heldTime = "time" in draft ? draft.time : "09:00";

  function switchKind(kind: ScheduleKind) {
    if (kind === draft.kind) return;
    switch (kind) {
      case "daily":
        onChange({ kind, time: heldTime });
        break;
      case "weekdays":
        onChange({ kind, time: heldTime });
        break;
      case "weekly":
        onChange({ kind, day: 1, time: heldTime });
        break;
      case "interval":
        onChange({ kind, minutes: 60 });
        break;
      case "custom":
        // Seed with the equivalent of the current draft, so Custom doubles
        // as "show me the cron for what I just picked".
        onChange({ kind, expression: scheduleFromDraft(draft) });
        break;
    }
  }

  const intervalUnit = draft.kind === "interval" && draft.minutes % 60 === 0 ? "hours" : "minutes";
  const intervalAmount =
    draft.kind === "interval" ? (intervalUnit === "hours" ? draft.minutes / 60 : draft.minutes) : 1;

  const preview =
    draft.kind === "custom"
      ? humanizeSchedule(draft.expression.trim())
      : humanizeSchedule(scheduleFromDraft(draft));
  const showPreview =
    draft.kind === "custom"
      ? draft.expression.trim().length > 0 && preview !== draft.expression.trim()
      : true;

  return (
    <div className="schedule-picker">
      <div className="schedule-picker-controls">
        <Select
          value={draft.kind}
          options={kindOptions(t)}
          placeholder={t("routines.schedule.placeholder")}
          ariaLabel={t("routines.schedule.typeLabel")}
          onChange={(kind) => switchKind(kind as ScheduleKind)}
        />

        {draft.kind === "weekly" ? (
          <Select
            value={String(draft.day)}
            options={dayOptions()}
            placeholder={t("routines.schedule.day")}
            ariaLabel={t("routines.schedule.dayOfWeek")}
            onChange={(day) => onChange({ ...draft, day: Number(day) })}
          />
        ) : null}

        {draft.kind === "daily" || draft.kind === "weekdays" || draft.kind === "weekly" ? (
          <input
            type="time"
            value={draft.time}
            aria-label={t("routines.schedule.time")}
            onChange={(event) => onChange({ ...draft, time: event.currentTarget.value })}
          />
        ) : null}

        {draft.kind === "interval" ? (
          <>
            <input
              type="number"
              min={1}
              value={intervalAmount}
              aria-label={t("routines.schedule.repeatEvery")}
              onChange={(event) => {
                const amount = Math.max(1, Math.floor(Number(event.currentTarget.value) || 1));
                onChange({
                  kind: "interval",
                  minutes: intervalUnit === "hours" ? amount * 60 : amount,
                });
              }}
            />
            <Select
              value={intervalUnit}
              options={unitOptions(t)}
              placeholder={t("routines.schedule.unit")}
              ariaLabel={t("routines.schedule.intervalUnit")}
              onChange={(unit) =>
                onChange({
                  kind: "interval",
                  minutes: unit === "hours" ? intervalAmount * 60 : intervalAmount,
                })
              }
            />
          </>
        ) : null}

        {draft.kind === "custom" ? (
          <input
            className="schedule-picker-custom"
            type="text"
            value={draft.expression}
            aria-label={t("routines.schedule.customLabel")}
            placeholder={t("routines.schedule.customPlaceholder")}
            onChange={(event) =>
              onChange({
                kind: "custom",
                expression: event.currentTarget.value,
              })
            }
          />
        ) : null}
      </div>

      {showPreview ? (
        <p className="schedule-picker-preview">{preview}</p>
      ) : draft.kind === "custom" ? (
        <p className="schedule-picker-preview">{t("routines.schedule.customHelp")}</p>
      ) : null}
    </div>
  );
}
