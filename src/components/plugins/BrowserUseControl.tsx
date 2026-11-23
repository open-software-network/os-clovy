import { IconGlobe } from "central-icons/IconGlobe";
import { useT } from "../../i18n";
import { useBrowserAccess } from "../../lib/browser-access";
import { useBrowserSessions } from "../../lib/use-browser-sessions";
import { Switch } from "../ui/Switch";
import { BrowserSessionControls } from "./BrowserSessionControls";

/** The capability owns a separate browser profile, never the personal browser. */
export function BrowserUseControl() {
  const t = useT();
  const access = useBrowserAccess();
  const { sessions, busy, error, act } = useBrowserSessions();
  return (
    <li className="connector-row browser-use-control">
      <span className="connector-logo" aria-hidden>
        <IconGlobe size={20} />
      </span>
      <div className="connector-main">
        <span className="connector-name">{t("settingsPanels.browserUse.name")}</span>
        <p className="connector-subtitle">{t("settingsPanels.browserUse.description")}</p>
      </div>
      <div className="connector-actions">
        <Switch
          checked={access.enabled === true}
          disabled={access.enabled === undefined || access.submitting}
          aria-label={t("settingsPanels.browserUse.enableAria")}
          onCheckedChange={(enabled) => void access.setGrant(enabled)}
        />
      </div>
      <div className="browser-use-details">
        <p className="connector-subtitle">{t("settingsPanels.browserUse.privacy")}</p>
        {sessions.map((session) => (
          <BrowserSessionControls
            key={session.sessionId}
            session={session}
            busy={busy !== undefined}
            onAction={(action) => void act(session.sessionId, action)}
          />
        ))}
        {access.error || error ? (
          <p className="settings-row-error" role="alert">
            {access.error || error}
          </p>
        ) : null}
      </div>
    </li>
  );
}
