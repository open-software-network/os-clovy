import { useRef } from "react";
import { useScrollFade } from "../../lib/use-scroll-fade";
import { useT } from "../../i18n";
import { useBrowserSessions } from "../../lib/use-browser-sessions";
import { BrowserSessionControls } from "../plugins/BrowserSessionControls";

/** Human control remains reachable when the user leaves the originating chat. */
export function BrowserSessionsTray() {
  const t = useT();
  const browser = useBrowserSessions();
  const listRef = useRef<HTMLDivElement>(null);
  const fade = useScrollFade(listRef);
  if (browser.sessions.length === 0 && !browser.actionError) return null;
  return (
    <aside className="connector-approvals" aria-label={t("settingsPanels.browserUse.name")}>
      <div ref={listRef} className="connector-approvals-list scroll-fade-mask" {...fade.props}>
        {browser.sessions.map((session) => (
          <BrowserSessionControls
            key={session.sessionId}
            session={session}
            busy={browser.busy !== undefined}
            onAction={(action) => void browser.act(session.sessionId, action)}
          />
        ))}
      </div>
      {browser.error ? (
        <p className="connector-approvals-error" role="alert">
          {browser.error}
        </p>
      ) : null}
      {browser.sessions.length === 0 && browser.actionError ? (
        <div className="agent-approval-actions">
          <button type="button" className="btn btn-ghost" onClick={browser.dismissActionError}>
            {t("common.close")}
          </button>
        </div>
      ) : null}
    </aside>
  );
}
