import { useEffect, useState } from "react";
import { formatClockTime, useT } from "../../i18n";
import {
  CODING_AGENT_NAMES,
  CODING_AGENT_SOURCE_FIELDS,
  type CodingAgentSourcesDto,
  type CodingAgentsStatusDto,
  codingAgentsStatus,
  onCodingAgentsUpdated,
} from "../../lib/coding-agents";
import { Switch } from "../ui/Switch";
import { RuntimeLogo } from "../brand/RuntimeLogo";

const STATUS_REFRESH_MS = 5_000;

/**
 * Settings, Activity: one switch per coding agent. The sources are part of
 * the activity settings, so the parent saves them with the rest; this section
 * only adds what the scanner reports (agent found on this Mac, last read).
 */
export function CodingAgentSourcesSection({
  sources,
  saving,
  onChange,
}: {
  sources: CodingAgentSourcesDto;
  saving: boolean;
  onChange: (next: CodingAgentSourcesDto) => void;
}) {
  const t = useT();
  const [status, setStatus] = useState<CodingAgentsStatusDto | null>(null);
  const anyEnabled = CODING_AGENT_SOURCE_FIELDS.some(([, field]) => sources[field]);

  // While a source is on, the scan runs every minute (and right after a
  // change), so the status is refreshed on updates and every few seconds.
  useEffect(() => {
    let active = true;
    const refresh = () => {
      void codingAgentsStatus()
        .then((next) => {
          if (active) setStatus(next);
        })
        .catch(() => {});
    };
    refresh();
    const timer = anyEnabled ? window.setInterval(refresh, STATUS_REFRESH_MS) : undefined;
    let unlisten: (() => void) | undefined;
    void onCodingAgentsUpdated(refresh).then((cleanup) => {
      if (active) unlisten = cleanup;
      else cleanup();
    });
    return () => {
      active = false;
      window.clearInterval(timer);
      unlisten?.();
    };
  }, [anyEnabled]);

  let statusLine: string | null = null;
  if (status && anyEnabled) {
    if (!status.databaseReady) {
      statusLine = t("codingAgents.settings.databaseUnavailable");
    } else if (status.lastError) {
      statusLine = t("codingAgents.settings.error", { message: status.lastError });
    } else if (status.lastScanAt) {
      const time = formatClockTime(status.lastScanAt);
      statusLine = t("codingAgents.settings.lastScan", { time });
    }
  }

  return (
    <section className="settings-group" aria-labelledby="coding-agents-heading">
      <h2 id="coding-agents-heading" className="settings-group-heading">
        {t("codingAgents.settings.title")}
      </h2>
      <p className="settings-group-description">{t("codingAgents.settings.description")}</p>
      <div className="settings-card">
        <div className="settings-rows">
          {CODING_AGENT_SOURCE_FIELDS.map(([id, field]) => {
            const name = CODING_AGENT_NAMES[id];
            const present = status?.sources.find((source) => source.id === id)?.present;
            return (
              <div key={id} className="settings-row">
                <div className="settings-row-info">
                  <h3 className="settings-row-title settings-row-title-with-mark">
                    <RuntimeLogo id={id} name={name} size={16} decorative />
                    {name}
                  </h3>
                  {present === undefined ? null : (
                    <p className="settings-row-description">
                      {present
                        ? t("codingAgents.settings.found")
                        : t("codingAgents.settings.notFound")}
                    </p>
                  )}
                </div>
                <div className="settings-row-control">
                  <Switch
                    checked={sources[field]}
                    disabled={saving}
                    aria-label={t("codingAgents.settings.toggleAria", { name })}
                    onCheckedChange={(enabled) => onChange({ ...sources, [field]: enabled })}
                  />
                </div>
              </div>
            );
          })}
          <div className="settings-row">
            <div className="settings-row-info">
              <p className="settings-row-description">{t("codingAgents.settings.summaries")}</p>
              {statusLine ? (
                <p className="settings-status" role="status">
                  {statusLine}
                </p>
              ) : null}
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
