import { TRIGGER_META, type TriggerDraft, DEFAULT_EVENT_LEAD_MINUTES } from "../../lib/connectors";
import type { ScheduleDraft } from "../../lib/routine-schedule";
import { Checkbox } from "../ui/Checkbox";
import { InlineNotice } from "../ui/InlineNotice";
import { Select } from "../ui/Select";
import { type TFunction, useT } from "../../i18n";
import { SchedulePicker } from "./SchedulePicker";

function sourceOptions(t: TFunction) {
  return [
    { value: "schedule", label: t("routines.trigger.onSchedule") },
    { value: "email_received", label: TRIGGER_META.email_received.label },
    { value: "event_upcoming", label: TRIGGER_META.event_upcoming.label },
  ];
}

/**
 * The routine editor's "When" control: a schedule (the existing picker), or a
 * connector event trigger (new mail, or a lead time before a meeting). Event
 * triggers never touch the schedule string. The routine stays active with a
 * far-future one-time schedule and the trigger daemon fires it, so the two
 * models stay separate here too.
 */
export function TriggerPicker({
  trigger,
  scheduleDraft,
  hasAccount,
  scopeWarning,
  onTriggerChange,
  onScheduleChange,
}: {
  trigger: TriggerDraft;
  scheduleDraft: ScheduleDraft;
  /** Whether any Google account is connected; event triggers require one. */
  hasAccount: boolean;
  /** Set when an account is connected but lacks the scope the selected trigger
   * needs, so the routine could be saved yet never fire. Null otherwise. */
  scopeWarning?: string | null;
  onTriggerChange: (trigger: TriggerDraft) => void;
  onScheduleChange: (draft: ScheduleDraft) => void;
}) {
  const t = useT();
  function switchSource(source: string) {
    if (source === trigger.source) return;
    if (source === "schedule") onTriggerChange({ source: "schedule" });
    else if (source === "email_received") onTriggerChange({ source: "email_received" });
    else {
      onTriggerChange({
        source: "event_upcoming",
        leadMinutes: DEFAULT_EVENT_LEAD_MINUTES,
        externalOnly: true,
      });
    }
  }

  return (
    <div className="trigger-picker">
      <div className="schedule-picker-controls">
        <Select
          value={trigger.source}
          options={sourceOptions(t)}
          placeholder={t("routines.trigger.placeholder")}
          ariaLabel={t("routines.trigger.typeLabel")}
          onChange={switchSource}
        />
        {trigger.source === "event_upcoming" ? (
          <>
            <input
              type="number"
              min={5}
              value={trigger.leadMinutes}
              aria-label={t("routines.trigger.leadLabel")}
              onChange={(event) => {
                const leadMinutes = Math.max(
                  5,
                  Math.floor(Number(event.currentTarget.value) || DEFAULT_EVENT_LEAD_MINUTES),
                );
                onTriggerChange({ ...trigger, leadMinutes });
              }}
            />
            <span className="trigger-picker-unit">{t("routines.trigger.minutesBefore")}</span>
          </>
        ) : null}
      </div>

      {trigger.source === "schedule" ? (
        <SchedulePicker draft={scheduleDraft} onChange={onScheduleChange} />
      ) : (
        <p className="schedule-picker-preview">{TRIGGER_META[trigger.source].description}</p>
      )}

      {trigger.source === "event_upcoming" ? (
        <label className="trigger-picker-toggle" htmlFor="trigger-external-only">
          <Checkbox
            id="trigger-external-only"
            checked={trigger.externalOnly}
            onChange={(event) =>
              onTriggerChange({ ...trigger, externalOnly: event.currentTarget.checked })
            }
          />
          {t("routines.trigger.externalOnly")}
        </label>
      ) : null}

      {trigger.source !== "schedule" && !hasAccount ? (
        <InlineNotice
          tone="warning"
          body={t("routines.trigger.needsAccount")}
          aria-label={t("routines.googleAccountRequired")}
        />
      ) : null}

      {trigger.source !== "schedule" && hasAccount && scopeWarning ? (
        <InlineNotice
          tone="warning"
          body={scopeWarning}
          aria-label={t("routines.trigger.moreAccess")}
        />
      ) : null}
    </div>
  );
}
