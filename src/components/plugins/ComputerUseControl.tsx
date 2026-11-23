import { IconCircleCheck } from "central-icons/IconCircleCheck";
import { IconCircleInfo } from "central-icons/IconCircleInfo";
import { IconExclamationCircle } from "central-icons/IconExclamationCircle";
import { IconStop } from "central-icons/IconStop";
import { IconTelevision } from "central-icons/IconTelevision";
import { useCallback, useEffect, useRef, useState } from "react";
import { t as translate, useT, useTRich } from "../../i18n";
import { messageFromError } from "../../lib/errors";
import {
  COMPUTER_USE_STATUS_CHANGED_EVENT,
  type ComputerUseStatusDto,
  computerUseRequestPermissions,
  computerUseStatus,
  computerUseStop,
  openPrivacySettings,
  setComputerUseGrant,
  setComputerUsePermissionDragBounds,
} from "../../lib/tauri";
import { InlineNotice } from "../ui/InlineNotice";
import { HoverTip } from "../ui/HoverTip";
import { Switch } from "../ui/Switch";

function statusLabel(status?: ComputerUseStatusDto) {
  if (!status) return translate("settingsPanels.computerUse.status.checking");
  if (!status.platformSupported) return translate("settingsPanels.computerUse.status.unavailable");
  if (!status.driverAvailable)
    return translate("settingsPanels.computerUse.status.driverUnavailable");
  if (!status.grantEnabled) return translate("settingsPanels.computerUse.status.off");
  if (status.ready) return translate("settingsPanels.computerUse.status.ready");
  if (status.platform === "windows")
    return translate("settingsPanels.computerUse.status.needsInteractiveDesktop");
  if (!status.accessibility || !status.screenRecording)
    return translate("settingsPanels.computerUse.status.needsMacosAccess");
  return translate("settingsPanels.computerUse.status.unavailable");
}

function requirementState(ready: boolean, enabled: boolean) {
  if (!enabled) return translate("settingsPanels.computerUse.requiredWhenEnabled");
  return ready
    ? translate("settingsPanels.computerUse.allowed")
    : translate("settingsPanels.computerUse.notAllowed");
}

type MacOSPermission = "accessibility" | "screenRecording";

const STATUS_POLL_INTERVAL_MS = 2000;
let pendingStatusRefresh: Promise<ComputerUseStatusDto> | undefined;
const pendingPermissionRequests = new Map<MacOSPermission, Promise<ComputerUseStatusDto>>();
let permissionRequestTail: Promise<unknown> = Promise.resolve();

function readComputerUseStatus() {
  if (pendingStatusRefresh) return pendingStatusRefresh;

  const request = computerUseStatus();
  pendingStatusRefresh = request;
  const clear = () => {
    if (pendingStatusRefresh === request) pendingStatusRefresh = undefined;
  };
  void request.then(clear, clear);
  return request;
}

function requestComputerUsePermission(permission: MacOSPermission) {
  const pending = pendingPermissionRequests.get(permission);
  if (pending) return pending;

  const request = permissionRequestTail
    .catch(() => undefined)
    .then(() => computerUseRequestPermissions());
  permissionRequestTail = request;
  pendingPermissionRequests.set(permission, request);
  const clear = () => {
    if (pendingPermissionRequests.get(permission) === request) {
      pendingPermissionRequests.delete(permission);
    }
  };
  void request.then(clear, clear);
  return request;
}

/**
 * Canonical front for the single native Computer use grant. Keeping management
 * in the Plugins provider list avoids a second preference surface and never implies that macOS
 * TCC access was granted by Clovy's switch.
 */
export function ComputerUseControl() {
  const t = useT();
  const tRich = useTRich();
  const [status, setStatus] = useState<ComputerUseStatusDto>();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string>();
  const permissionDragRef = useRef<HTMLButtonElement>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await readComputerUseStatus());
    } catch (error) {
      setMessage(messageFromError(error));
    }
  }, []);

  useEffect(() => {
    let active = true;
    const load = async () => {
      try {
        const next = await readComputerUseStatus();
        if (active) setStatus(next);
      } catch (error) {
        if (active) setMessage(messageFromError(error));
      }
    };
    void load();
    const onFocus = () => void load();
    const onChanged = () => void load();
    window.addEventListener("focus", onFocus);
    window.addEventListener(COMPUTER_USE_STATUS_CHANGED_EVENT, onChanged);
    return () => {
      active = false;
      window.removeEventListener("focus", onFocus);
      window.removeEventListener(COMPUTER_USE_STATUS_CHANGED_EVENT, onChanged);
    };
  }, []);

  useEffect(() => {
    if (!status?.grantEnabled || status.ready) return;
    let active = true;
    let timer: number | undefined;
    const clearTimer = () => {
      if (timer !== undefined) window.clearTimeout(timer);
      timer = undefined;
    };
    const schedule = () => {
      clearTimer();
      if (!active || document.visibilityState !== "visible") return;
      timer = window.setTimeout(async () => {
        await refresh();
        schedule();
      }, STATUS_POLL_INTERVAL_MS);
    };
    const onVisibilityChange = () => {
      clearTimer();
      if (document.visibilityState === "visible") {
        void refresh().then(schedule);
      }
    };

    schedule();
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      active = false;
      clearTimer();
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [refresh, status?.grantEnabled, status?.ready]);

  const publish = useCallback((next: ComputerUseStatusDto) => {
    setStatus(next);
    window.dispatchEvent(new Event(COMPUTER_USE_STATUS_CHANGED_EVENT));
  }, []);

  const toggleGrant = useCallback(
    async (enabled: boolean) => {
      setBusy(true);
      setMessage(undefined);
      try {
        const next = await setComputerUseGrant(enabled);
        publish(next);
        setMessage(enabled ? undefined : t("settingsPanels.computerUse.offMessage"));
      } catch (error) {
        setMessage(messageFromError(error));
        await refresh();
      } finally {
        setBusy(false);
      }
    },
    [publish, refresh, t],
  );

  const stop = useCallback(async () => {
    setBusy(true);
    setMessage(undefined);
    try {
      await computerUseStop();
      setMessage(t("settingsPanels.computerUse.stoppedMessage"));
      await refresh();
    } catch (error) {
      setMessage(messageFromError(error));
    } finally {
      setBusy(false);
    }
  }, [refresh, t]);

  const openPermissionSettings = useCallback(
    async (pane: "accessibility" | "screenRecording") => {
      try {
        // Ask first so macOS creates the responsible app entry before the
        // matching privacy pane appears. Coalesce repeated clicks because a
        // cold signed-helper launch and TCC probe can take several seconds.
        publish(await requestComputerUsePermission(pane));
        await openPrivacySettings(pane);
      } catch (error) {
        setMessage(messageFromError(error));
      }
    },
    [publish],
  );

  const enabled = status?.grantEnabled === true;
  const supported = status?.platformSupported !== false;
  const windows = status?.platform === "windows";
  const driverReady = status?.driverAvailable !== false;
  const statusErrorShownInline = supported && !driverReady && status !== undefined;
  const permissionsMissing =
    !windows &&
    enabled &&
    status !== undefined &&
    (!status.accessibility || !status.screenRecording);
  const nextPermission: MacOSPermission = status?.accessibility
    ? "screenRecording"
    : "accessibility";
  const permissionStep = nextPermission === "accessibility" ? 1 : 2;

  useEffect(() => {
    const element = permissionDragRef.current;
    if (!permissionsMissing || !element) {
      void setComputerUsePermissionDragBounds(null);
      return;
    }

    const publishBounds = () => {
      const bounds = element.getBoundingClientRect();
      void setComputerUsePermissionDragBounds(
        {
          x: bounds.x,
          y: bounds.y,
          width: bounds.width,
          height: bounds.height,
        },
        nextPermission === "accessibility" ? "helper" : "host",
      ).catch((error) => setMessage(messageFromError(error)));
    };
    publishBounds();

    const observer =
      typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(publishBounds);
    observer?.observe(element);
    window.addEventListener("resize", publishBounds);
    window.addEventListener("scroll", publishBounds, true);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", publishBounds);
      window.removeEventListener("scroll", publishBounds, true);
      void setComputerUsePermissionDragBounds(null);
    };
  }, [nextPermission, permissionsMissing]);

  return (
    <li className="connector-row computer-use-control" data-state={status?.state}>
      <span className="connector-logo" aria-hidden>
        <IconTelevision size={20} />
      </span>
      <div className="connector-main">
        <span className="computer-use-title-line">
          <span className="connector-name">{t("settingsPanels.computerUse.name")}</span>
          <HoverTip tip={t("settingsPanels.computerUse.privacyTip")} width={360}>
            <button
              type="button"
              className="settings-row-info-affordance"
              aria-label={t("settingsPanels.computerUse.privacyAria")}
            >
              <IconCircleInfo size={13} ariaHidden />
            </button>
          </HoverTip>
        </span>
        <p className="connector-subtitle">{t("settingsPanels.computerUse.subtitle")}</p>
      </div>
      <div className="connector-actions">
        <span className="computer-use-state-label">{statusLabel(status)}</span>
        <Switch
          checked={enabled}
          disabled={busy || !supported || status === undefined || (!enabled && !driverReady)}
          aria-label={t("settingsPanels.computerUse.enableAria")}
          onCheckedChange={(next) => void toggleGrant(next)}
        />
      </div>

      <div className="computer-use-details">
        {!supported ? (
          <InlineNotice
            tone="info"
            icon={<IconExclamationCircle size={16} />}
            body={t("settingsPanels.computerUse.supportedPlatforms")}
          />
        ) : null}

        {supported && !driverReady && status ? (
          <InlineNotice
            tone="destructive"
            icon={<IconExclamationCircle size={16} />}
            eyebrow={t("settingsPanels.computerUse.driverUnavailableTitle")}
            body={status.error || t("settingsPanels.computerUse.driverUnavailableBody")}
          />
        ) : null}

        {enabled ? (
          <div className="computer-use-setup">
            <p className="computer-use-message">{t("settingsPanels.computerUse.inputMode")}</p>
            {windows ? (
              <InlineNotice
                tone="info"
                body={
                  status?.ready
                    ? t("settingsPanels.computerUse.windowsReady")
                    : status?.error || t("settingsPanels.computerUse.windowsRequirements")
                }
              />
            ) : null}
            {permissionsMissing ? (
              <section
                className="computer-use-permission-assistant"
                aria-labelledby="add-clovy-macos"
              >
                <div className="computer-use-permission-assistant-header">
                  <div className="computer-use-permission-assistant-copy">
                    <span className="computer-use-permission-step">
                      {t("settingsPanels.computerUse.step", {
                        step: String(permissionStep),
                        total: "2",
                      })}
                    </span>
                    <h4 id="add-clovy-macos">
                      {nextPermission === "accessibility"
                        ? t("settingsPanels.computerUse.allowAccessibility")
                        : t("settingsPanels.computerUse.allowScreenRecording")}
                    </h4>
                    {nextPermission === "accessibility" ? (
                      <p>
                        {tRich("settingsPanels.computerUse.accessibilityHelp", {
                          strong: (chunks) => <strong>{chunks}</strong>,
                        })}
                      </p>
                    ) : (
                      <p>
                        {tRich("settingsPanels.computerUse.screenRecordingHelp", {
                          strong: (chunks) => <strong>{chunks}</strong>,
                        })}
                      </p>
                    )}
                  </div>
                  <button
                    type="button"
                    className="btn btn-primary computer-use-permission-primary-action"
                    onClick={() => void openPermissionSettings(nextPermission)}
                  >
                    {nextPermission === "accessibility"
                      ? t("settingsPanels.computerUse.openAccessibilitySettings")
                      : t("settingsPanels.computerUse.openScreenRecordingSettings")}
                  </button>
                </div>

                <div className="computer-use-permission-helper">
                  <div className="computer-use-permission-helper-copy">
                    <strong>
                      {nextPermission === "accessibility"
                        ? t("settingsPanels.computerUse.driverNotListed")
                        : t("settingsPanels.computerUse.clovyNotListed")}
                    </strong>
                    <p>
                      {nextPermission === "accessibility"
                        ? t("settingsPanels.computerUse.dragHelperBelow")
                        : t("settingsPanels.computerUse.dragClovyBelow")}
                    </p>
                  </div>
                  <button
                    ref={permissionDragRef}
                    type="button"
                    className="computer-use-permission-drag-card"
                    aria-label={t("settingsPanels.computerUse.dragAria", {
                      name:
                        nextPermission === "accessibility" ? "Clovy Computer Use Driver" : "Clovy",
                    })}
                    onClick={() => void openPermissionSettings(nextPermission)}
                  >
                    <span className="computer-use-permission-drag-icon" aria-hidden>
                      <IconTelevision size={20} />
                    </span>
                    <span className="computer-use-permission-drag-copy">
                      <strong>
                        {nextPermission === "accessibility" ? "Clovy Computer Use Driver" : "Clovy"}
                      </strong>
                      <span>{t("settingsPanels.computerUse.dragInto")}</span>
                    </span>
                  </button>
                </div>
              </section>
            ) : null}

            {!windows ? (
              <section
                className="computer-use-requirements"
                aria-labelledby="computer-use-requirements"
              >
                <h4 id="computer-use-requirements" className="computer-use-requirements-heading">
                  {t("settingsPanels.computerUse.setupProgress")}
                </h4>
                <div className="computer-use-requirement">
                  <span
                    className="computer-use-requirement-icon"
                    data-ready={status?.accessibility}
                  >
                    {status?.accessibility ? (
                      <IconCircleCheck size={15} aria-hidden />
                    ) : (
                      <IconExclamationCircle size={15} aria-hidden />
                    )}
                  </span>
                  <span className="computer-use-requirement-copy">
                    <strong>{t("settingsPanels.computerUse.accessibility")}</strong>
                    <span>{requirementState(status?.accessibility === true, enabled)}</span>
                  </span>
                </div>
                <div className="computer-use-requirement">
                  <span
                    className="computer-use-requirement-icon"
                    data-ready={status?.screenRecording}
                  >
                    {status?.screenRecording ? (
                      <IconCircleCheck size={15} aria-hidden />
                    ) : (
                      <IconExclamationCircle size={15} aria-hidden />
                    )}
                  </span>
                  <span className="computer-use-requirement-copy">
                    <strong>{t("settingsPanels.computerUse.screenRecording")}</strong>
                    <span>{requirementState(status?.screenRecording === true, enabled)}</span>
                  </span>
                </div>
              </section>
            ) : null}

            <div className="computer-use-actions">
              <button
                type="button"
                className="btn btn-ghost"
                disabled={busy}
                onClick={() => void stop()}
              >
                <IconStop size={14} aria-hidden />
                {t("settingsPanels.computerUse.stopTask")}
              </button>
            </div>
          </div>
        ) : null}

        {message ||
        (status?.state !== "permission_missing" && !statusErrorShownInline && status?.error) ? (
          <p className="computer-use-message" role="status">
            {message || status?.error}
          </p>
        ) : null}
      </div>
    </li>
  );
}
