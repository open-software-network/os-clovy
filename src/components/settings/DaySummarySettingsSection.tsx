import { useT } from "../../i18n";
import type { ActivitySettingsDto } from "../../lib/activity-capture";
import { Switch } from "../ui/Switch";

type DaySummarySettingsSectionProps = {
  settings: ActivitySettingsDto;
  saving: boolean;
  onSave: (settings: ActivitySettingsDto) => void;
};

/** Settings, Activity: when the day summary runs and how it notifies. */
export function DaySummarySettingsSection({
  settings,
  saving,
  onSave,
}: DaySummarySettingsSectionProps) {
  const t = useT();
  const { notifications } = settings;
  const quiet = notifications.quietHours;
  const saveQuiet = (next: Partial<typeof quiet>) =>
    onSave({
      ...settings,
      notifications: { ...notifications, quietHours: { ...quiet, ...next } },
    });

  return (
    <section className="settings-group" aria-labelledby="activity-day-summary-heading">
      <h2 id="activity-day-summary-heading" className="settings-group-heading">
        {t("dayIntelligence.settings.title")}
      </h2>
      <p className="settings-group-description">{t("dayIntelligence.settings.description")}</p>
      <div className="settings-card">
        <div className="settings-rows">
          <div className="settings-row">
            <div className="settings-row-info">
              <h3 className="settings-row-title">{t("dayIntelligence.settings.time")}</h3>
              <p className="settings-row-description">
                {t("dayIntelligence.settings.timeDescription")}
              </p>
            </div>
            <div className="settings-row-control">
              <input
                type="time"
                className="dialog-input day-summary-time-input"
                value={settings.daySummary.time}
                aria-label={t("dayIntelligence.settings.time")}
                disabled={saving}
                onChange={(event) =>
                  onSave({ ...settings, daySummary: { time: event.target.value } })
                }
              />
            </div>
          </div>

          <div className="settings-row">
            <div className="settings-row-info">
              <h3 className="settings-row-title">{t("dayIntelligence.settings.notifications")}</h3>
              <p className="settings-row-description">
                {t("dayIntelligence.settings.notificationsDescription")}
              </p>
            </div>
            <div className="settings-row-control">
              <Switch
                checked={notifications.enabled}
                disabled={saving}
                aria-label={t("dayIntelligence.settings.notifications")}
                onCheckedChange={(enabled) =>
                  onSave({ ...settings, notifications: { ...notifications, enabled } })
                }
              />
            </div>
          </div>

          <div className="settings-row">
            <div className="settings-row-info">
              <h3 className="settings-row-title">{t("dayIntelligence.settings.quietHours")}</h3>
              <p className="settings-row-description">
                {t("dayIntelligence.settings.quietHoursDescription")}
              </p>
            </div>
            <div className="settings-row-control">
              <Switch
                checked={quiet.enabled}
                disabled={saving || !notifications.enabled}
                aria-label={t("dayIntelligence.settings.quietHours")}
                onCheckedChange={(enabled) => saveQuiet({ enabled })}
              />
            </div>
          </div>

          {quiet.enabled ? (
            <>
              <div className="settings-row">
                <div className="settings-row-info">
                  <h3 className="settings-row-title">{t("dayIntelligence.settings.quietStart")}</h3>
                </div>
                <div className="settings-row-control">
                  <input
                    type="time"
                    className="dialog-input day-summary-time-input"
                    value={quiet.start}
                    aria-label={t("dayIntelligence.settings.quietStart")}
                    disabled={saving || !notifications.enabled}
                    onChange={(event) => saveQuiet({ start: event.target.value })}
                  />
                </div>
              </div>
              <div className="settings-row">
                <div className="settings-row-info">
                  <h3 className="settings-row-title">{t("dayIntelligence.settings.quietEnd")}</h3>
                </div>
                <div className="settings-row-control">
                  <input
                    type="time"
                    className="dialog-input day-summary-time-input"
                    value={quiet.end}
                    aria-label={t("dayIntelligence.settings.quietEnd")}
                    disabled={saving || !notifications.enabled}
                    onChange={(event) => saveQuiet({ end: event.target.value })}
                  />
                </div>
              </div>
            </>
          ) : null}
        </div>
      </div>
    </section>
  );
}
