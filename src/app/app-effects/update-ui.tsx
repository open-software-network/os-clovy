import { IconChevronRightSmall } from "central-icons/IconChevronRightSmall";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import type { CSSProperties } from "react";
import { type TFunction, useT } from "../../i18n";
import type { ClovyUpdate } from "../../lib/updater";
import { ClovyMark } from "../../components/brand/ClovyLogo";
import { Spinner } from "../../components/ui/Spinner";
import {
  UP_TO_DATE_STATUS,
  type UpdateInstallProgress,
  type UpdatePromptPayload,
} from "../update-decision";

export type UpdateHubProps = {
  // The newer version found by the last check, until the owner postpones it.
  offer: UpdatePromptPayload<ClovyUpdate> | null;
  // The owner chose to update and has not postponed since.
  accepted: boolean;
  // Installed; only the restart is left.
  ready: boolean;
  // Accepted while a recording or dictation runs: the next step waits for it.
  waitingForCapture: boolean;
  status: string | null;
  failed: boolean;
  statusLeaving: boolean;
  checking: boolean;
  preparing: boolean;
  relaunching: boolean;
  progress: UpdateInstallProgress | null;
  onInstall: () => void;
  onPostpone: () => void;
  onRelaunch: () => void;
  onDismissStatus: () => void;
};

// The sidebar footer card for every update state. Exactly one card shows: the
// offer, the wait for a recording to end, the restart, or a transient status
// (checking, downloading, up to date, failure).
export function UpdateHub({
  offer,
  accepted,
  ready,
  waitingForCapture,
  status,
  failed,
  statusLeaving,
  checking,
  preparing,
  relaunching,
  progress,
  onInstall,
  onPostpone,
  onRelaunch,
  onDismissStatus,
}: UpdateHubProps) {
  if (offer && waitingForCapture) {
    return <UpdateWaitCard ready={ready} onPostpone={onPostpone} />;
  }
  if (offer && (ready || relaunching)) {
    return (
      <UpdateRelaunchCard
        payload={offer}
        status={status}
        failed={failed}
        relaunching={relaunching}
        onRelaunch={onRelaunch}
      />
    );
  }
  if (offer && !accepted && !preparing) {
    return <UpdateOfferCard payload={offer} onInstall={onInstall} onPostpone={onPostpone} />;
  }

  if (!status) return null;
  return (
    <UpdateStatusCard
      status={status}
      failed={failed}
      leaving={statusLeaving}
      checking={checking}
      preparing={preparing}
      progress={progress}
      onDismiss={onDismissStatus}
    />
  );
}

function UpdateOfferCard({
  payload,
  onInstall,
  onPostpone,
}: {
  payload: UpdatePromptPayload<ClovyUpdate>;
  onInstall: () => void;
  onPostpone: () => void;
}) {
  const t = useT();
  return (
    <aside
      className="update-popover update-card"
      role="status"
      aria-live="polite"
      aria-label={t("app.update.offerAria", { version: payload.version })}
    >
      <div className="update-status-row">
        <span className="update-status-mark" aria-hidden>
          <ClovyMark variant="mono" />
        </span>
        <span className="update-card-title">
          {t("app.update.available", { version: payload.version })}
        </span>
      </div>
      <div className="update-card-actions">
        <button type="button" className="btn btn-ghost update-card-action" onClick={onPostpone}>
          {t("app.update.later")}
        </button>
        <button type="button" className="btn btn-primary update-card-action" onClick={onInstall}>
          {t("app.update.install")}
        </button>
      </div>
    </aside>
  );
}

function UpdateWaitCard({ ready, onPostpone }: { ready: boolean; onPostpone: () => void }) {
  const t = useT();
  return (
    <aside className="update-popover update-card" role="status" aria-live="polite">
      <div className="update-status-row">
        <span className="update-status-mark" aria-hidden>
          <ClovyMark variant="mono" />
        </span>
        <span className="update-card-text">
          {ready ? t("app.update.restartAfterCapture") : t("app.update.waitingForCapture")}
        </span>
      </div>
      <div className="update-card-actions">
        <button type="button" className="btn btn-ghost update-card-action" onClick={onPostpone}>
          {t("app.update.later")}
        </button>
      </div>
    </aside>
  );
}

function UpdateRelaunchCard({
  payload,
  status,
  failed,
  relaunching,
  onRelaunch,
}: {
  payload: UpdatePromptPayload<ClovyUpdate>;
  status: string | null;
  failed: boolean;
  relaunching: boolean;
  onRelaunch: () => void;
}) {
  const t = useT();
  const meta = displayedUpdateStatus(status, t) ?? updateVersionLabel(payload.version);

  return (
    <aside className="update-popover" role={failed ? "alert" : "status"} aria-live="polite">
      <button
        type="button"
        className="update-relaunch-card"
        disabled={relaunching}
        aria-label={t("app.update.relaunchAria", { version: payload.version })}
        onClick={onRelaunch}
      >
        {/* One motion cue per card: while relaunching the mark slot swaps to the
         * dot spinner and the title stays plain text. */}
        <span className="update-relaunch-mark" aria-hidden>
          {relaunching ? <Spinner size="sm" aria-hidden /> : <ClovyMark variant="mono" />}
        </span>
        <span className="update-relaunch-copy">
          <span className="update-relaunch-title">
            {relaunching ? t("app.update.relaunching") : t("app.update.relaunch")}
          </span>
          <span className={status ? "update-relaunch-status" : undefined}>{meta}</span>
        </span>
        {!relaunching && (
          <IconChevronRightSmall className="update-relaunch-arrow" size={16} aria-hidden />
        )}
      </button>
    </aside>
  );
}

function UpdateStatusCard({
  status,
  failed,
  leaving,
  checking,
  preparing,
  progress,
  onDismiss,
}: {
  status: string;
  failed: boolean;
  leaving: boolean;
  checking: boolean;
  preparing: boolean;
  progress: UpdateInstallProgress | null;
  onDismiss: () => void;
}) {
  const t = useT();
  const percent = updateProgressPercent(progress);
  const progressWidth =
    progress?.state === "installing" && percent === undefined ? "100%" : `${percent ?? 0}%`;
  // Explicit flags, never string-sniffed: checking covers the manual check
  // round-trip, preparing covers download and install. The spinner is
  // decorative; the status text announces the state to assistive technology.
  const busy = checking || preparing;

  return (
    <aside
      className={
        failed ? "update-popover update-card update-card-failed" : "update-popover update-card"
      }
      data-leaving={leaving || undefined}
      role={failed ? "alert" : "status"}
      aria-live="polite"
    >
      <div className="update-status-row">
        <span className="update-status-mark" aria-hidden>
          {busy ? <Spinner size="sm" aria-hidden /> : <ClovyMark variant="mono" />}
        </span>
        <span
          className={failed ? "update-status-text update-status-text-failed" : "update-status-text"}
        >
          {displayedUpdateStatus(status, t)}
        </span>
        <button
          type="button"
          className="update-status-close"
          aria-label={preparing ? t("app.update.hideProgress") : t("app.update.dismissStatus")}
          onClick={onDismiss}
        >
          <IconCrossSmall size={12} aria-hidden />
        </button>
      </div>
      {progress ? (
        <div className="update-progress" aria-hidden>
          <div className="update-progress-track">
            <div
              className="update-progress-fill"
              style={
                {
                  "--update-progress-clip": `calc(100% - max(var(--sp-2), ${progressWidth}))`,
                } as CSSProperties
              }
            />
          </div>
          {percent !== undefined ? (
            <span className="update-progress-percent update-digit-group">
              {/* Each digit keyed by position+character: only a digit whose
               * character changed remounts and replays the pop-in, so the ones
               * digit ticks each percent while the tens digit only rolls over.
               * The % sign stays static. */}
              {String(percent)
                .split("")
                .map((char, i) => (
                  // biome-ignore lint/suspicious/noArrayIndexKey: the position-plus-character key is the mechanism; a changed digit remounts to replay the pop-in.
                  <span key={`${i}-${char}`} className="update-digit">
                    {char}
                  </span>
                ))}
              <span>%</span>
            </span>
          ) : null}
        </div>
      ) : null}
    </aside>
  );
}

function updateProgressPercent(progress: UpdateInstallProgress | null) {
  if (!progress?.contentLength || progress.contentLength <= 0) return undefined;
  return Math.min(
    100,
    Math.round(((progress.downloadedBytes ?? 0) / progress.contentLength) * 100),
  );
}

// The up-to-date status is an English sentinel in state (the auto-dismiss
// reducer matches on it); translate it only for display.
function displayedUpdateStatus(status: string | null, t: TFunction) {
  return status === UP_TO_DATE_STATUS ? t("app.update.upToDate") : status;
}

function updateVersionLabel(version: string) {
  return version.startsWith("v") ? version : `v${version}`;
}
