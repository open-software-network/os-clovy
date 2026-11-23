import { useT } from "../../i18n";
import type { BrowserSession } from "../../lib/tauri";

export function BrowserSessionControls({
  session,
  busy,
  onAction,
}: {
  session: BrowserSession;
  busy: boolean;
  onAction: (action: "takeover" | "resume" | "stop") => void;
}) {
  const t = useT();
  const waitingForWindow = session.suspended && session.mode === "background";
  return (
    <section className="browser-session-control" aria-label={t("settingsPanels.browserUse.name")}>
      <p className="browser-session-state" role="status">
        {t(
          session.suspended
            ? waitingForWindow
              ? "settingsPanels.browserUse.pausedBackground"
              : "settingsPanels.browserUse.paused"
            : session.mode === "background"
              ? "settingsPanels.browserUse.background"
              : "settingsPanels.browserUse.visible",
        )}
      </p>
      {session.suspended ? (
        <p className="connector-subtitle">
          {t(
            waitingForWindow
              ? "settingsPanels.browserUse.pausedBackgroundHelp"
              : "settingsPanels.browserUse.pausedHelp",
          )}
        </p>
      ) : null}
      <div className="agent-approval-actions">
        <button
          type="button"
          className="btn btn-secondary"
          disabled={busy}
          onClick={() => onAction(session.suspended ? "resume" : "takeover")}
        >
          {t(
            session.suspended
              ? "settingsPanels.browserUse.resume"
              : "settingsPanels.browserUse.takeover",
          )}
        </button>
        <button type="button" className="btn btn-ghost" onClick={() => onAction("stop")}>
          {t("settingsPanels.browserUse.stop")}
        </button>
      </div>
    </section>
  );
}
