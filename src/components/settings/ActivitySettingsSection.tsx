import { useEffect, useState } from "react";
import { IconCircleCheck } from "central-icons/IconCircleCheck";
import { IconCircleQuestionmark } from "central-icons/IconCircleQuestionmark";
import { IconExclamationCircle } from "central-icons/IconExclamationCircle";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { IconTrashCan } from "central-icons/IconTrashCan";
import {
  activityDebugExport,
  activityRecreateDatabase,
  activityRequestPermission,
  activitySaveSettings,
  activitySetPaused,
  activityStatus,
  normalizeDomain,
  onActivityState,
  type ActivityDebugExportDto,
  type ActivityPermissionState,
  type ActivitySettingsDto,
  type ActivityStatusDto,
} from "../../lib/activity-capture";
import { openPrivacySettings } from "../../lib/tauri";
import { isMacLikePlatform } from "../../lib/platform";
import { formatDate, type MessageKey, useT } from "../../i18n";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { Switch } from "../ui/Switch";
import { DaySummarySettingsSection } from "./DaySummarySettingsSection";
import { SettingsPageHeader } from "./AppSettings";
import { CodingAgentSourcesSection } from "./CodingAgentSourcesSection";

const ISO_WEEKDAYS = [1, 2, 3, 4, 5, 6, 7] as const;

export function ActivitySettingsSection() {
  const t = useT();
  const [status, setStatus] = useState<ActivityStatusDto | null>(null);
  const [saving, setSaving] = useState(false);
  const [keyMissingConfirmOpen, setKeyMissingConfirmOpen] = useState(false);
  const [newAppName, setNewAppName] = useState("");
  const [newDomainName, setNewDomainName] = useState("");
  const [debugResult, setDebugResult] = useState<ActivityDebugExportDto | null>(null);
  const [debugExporting, setDebugExporting] = useState(false);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    void activityStatus()
      .then(setStatus)
      .catch(() => {});

    void onActivityState((next) => {
      setStatus(next);
    }).then((cleanup) => {
      if (active) unlisten = cleanup;
      else cleanup();
    });

    const handleFocus = () => {
      void activityStatus()
        .then(setStatus)
        .catch(() => {});
    };
    window.addEventListener("focus", handleFocus);

    return () => {
      active = false;
      unlisten?.();
      window.removeEventListener("focus", handleFocus);
    };
  }, []);
  if (!status?.supported) {
    return null;
  }

  const { settings, state, permissions } = status;

  async function saveSettings(nextSettings: ActivitySettingsDto) {
    setSaving(true);
    try {
      const updated = await activitySaveSettings(nextSettings);
      setStatus(updated);
    } finally {
      setSaving(false);
    }
  }

  function handleToggleEnabled(enabled: boolean) {
    void saveSettings({ ...settings, enabled });
  }

  // Only the user's own pause can be resumed here. Automatic pauses (work
  // hours, low disk, protected video) keep the button as "Pause capture", which
  // adds a manual pause on top, matching the menu bar.
  function handleTogglePaused() {
    void activitySetPaused(!status?.manualPause).then(setStatus);
  }

  function handleRequestPermission(
    permission: "accessibility" | "screenRecording" | "inputMonitoring",
  ) {
    void activityRequestPermission(permission).then(setStatus);
  }

  function handleOpenSettings(pane: "accessibility" | "screenRecording" | "inputMonitoring") {
    void openPrivacySettings(pane);
  }

  function handleAddApp() {
    const name = newAppName.trim();
    if (!name) return;
    const exists = settings.ignoredApps.some((app) => app.toLowerCase() === name.toLowerCase());
    if (!exists) {
      void saveSettings({
        ...settings,
        ignoredApps: [...settings.ignoredApps, name],
      });
    }
    setNewAppName("");
  }

  function handleRemoveApp(appName: string) {
    void saveSettings({
      ...settings,
      ignoredApps: settings.ignoredApps.filter(
        (app) => app.toLowerCase() !== appName.toLowerCase(),
      ),
    });
  }

  function handleAddDomain() {
    const raw = newDomainName.trim();
    if (!raw) return;
    const domain = normalizeDomain(raw);
    if (!domain) return;
    const exists = settings.ignoredDomains.some((d) => d.toLowerCase() === domain.toLowerCase());
    if (!exists) {
      void saveSettings({
        ...settings,
        ignoredDomains: [...settings.ignoredDomains, domain],
      });
    }
    setNewDomainName("");
  }

  function handleRemoveDomain(domainName: string) {
    void saveSettings({
      ...settings,
      ignoredDomains: settings.ignoredDomains.filter(
        (d) => d.toLowerCase() !== domainName.toLowerCase(),
      ),
    });
  }

  function handleToggleWeekday(day: number) {
    const exists = settings.workHours.days.includes(day);
    const updatedDays = exists
      ? settings.workHours.days.filter((d) => d !== day)
      : [...settings.workHours.days, day].sort((a, b) => a - b);
    void saveSettings({
      ...settings,
      workHours: {
        ...settings.workHours,
        days: updatedDays,
      },
    });
  }

  function handleDebugExport() {
    setDebugExporting(true);
    void activityDebugExport()
      .then(setDebugResult)
      .finally(() => setDebugExporting(false));
  }

  const isPaused = status.manualPause;
  const needsPermissions = state.kind === "needsPermissions";
  const missingRequiredList = needsPermissions ? state.missing : [];

  let statusText = "";
  if (state.kind === "active") {
    statusText = t("activity.status.active");
  } else if (state.kind === "off") {
    statusText = t("activity.status.off");
  } else if (state.kind === "paused") {
    switch (state.reason) {
      case "workHours":
        statusText = t("activity.status.pausedWorkHours");
        break;
      case "lowDisk":
        statusText = t("activity.status.pausedLowDisk");
        break;
      case "protectedVideo":
        statusText = t("activity.status.pausedProtectedVideo");
        break;
      default:
        statusText = t("activity.status.pausedManual");
        break;
    }
  } else if (state.kind === "needsPermissions") {
    const missingNames = state.missing
      .map((m) =>
        m === "accessibility"
          ? t("activity.permissions.accessibility.title")
          : t("activity.permissions.screenRecording.title"),
      )
      .join(", ");
    statusText = t("activity.status.needsPermissions", { missing: missingNames });
  } else if (state.kind === "keyMissing") {
    statusText = t("activity.status.keyMissing");
  } else if (state.kind === "error") {
    statusText = t("activity.status.error", { message: state.message });
  }

  let formattedLastFrame: string | null = null;
  if (status.lastFrameAt) {
    try {
      const d = new Date(status.lastFrameAt);
      formattedLastFrame = !Number.isNaN(d.getTime())
        ? formatDate(d, { hour: "2-digit", minute: "2-digit", second: "2-digit" })
        : status.lastFrameAt;
    } catch {
      formattedLastFrame = status.lastFrameAt;
    }
  }

  return (
    <>
      <section className="settings-group" aria-labelledby="activity-heading">
        <SettingsPageHeader
          id="activity-heading"
          title={t("activity.title")}
          blurb={t("activity.blurb")}
        />

        <div className="settings-card">
          <div className="settings-rows">
            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("activity.capture.title")}</h3>
                <p className="settings-row-description">{t("activity.capture.description")}</p>
              </div>
              <div className="settings-row-control">
                <Switch
                  checked={settings.enabled}
                  disabled={saving}
                  aria-label={t("activity.capture.title")}
                  onCheckedChange={handleToggleEnabled}
                />
              </div>
            </div>

            <div className="settings-row">
              <div className="settings-row-info">
                <p className="settings-status" role="status">
                  {statusText}
                  {formattedLastFrame
                    ? ` • ${t("activity.status.lastFrameAt", { time: formattedLastFrame })}`
                    : null}
                </p>
              </div>
              <div className="settings-row-control">
                <button
                  type="button"
                  className="btn btn-secondary"
                  disabled={!settings.enabled || saving}
                  onClick={handleTogglePaused}
                >
                  {isPaused ? t("activity.controls.resume") : t("activity.controls.pause")}
                </button>
              </div>
            </div>
          </div>
        </div>
      </section>

      {state.kind === "keyMissing" ? (
        <section className="settings-group" aria-labelledby="activity-key-missing-heading">
          <h2 id="activity-key-missing-heading" className="settings-group-heading">
            {t("activity.keyMissing.title")}
          </h2>
          <div className="settings-card">
            <div className="settings-rows">
              <div className="settings-row">
                <div className="settings-row-info">
                  <p className="settings-row-description">{t("activity.keyMissing.description")}</p>
                </div>
                <div className="settings-row-control">
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={() => setKeyMissingConfirmOpen(true)}
                  >
                    {t("activity.keyMissing.recreate")}
                  </button>
                </div>
              </div>
            </div>
          </div>
          <ConfirmDialog
            open={keyMissingConfirmOpen}
            title={t("activity.keyMissing.confirmTitle")}
            description={t("activity.keyMissing.confirmDescription")}
            confirmLabel={t("activity.keyMissing.confirmAction")}
            cancelLabel={t("activity.keyMissing.cancelAction")}
            destructive
            onClose={() => setKeyMissingConfirmOpen(false)}
            onConfirm={async () => {
              const recreated = await activityRecreateDatabase();
              setStatus(recreated);
              setKeyMissingConfirmOpen(false);
            }}
          />
        </section>
      ) : null}

      {isMacLikePlatform() ? (
        <section className="settings-group" aria-labelledby="activity-permissions-heading">
          <h2 id="activity-permissions-heading" className="settings-group-heading">
            {t("activity.permissions.title")}
          </h2>
          <p className="settings-group-description">{t("activity.permissions.description")}</p>
          <div className="settings-card">
            <div className="settings-rows">
              <PermissionItemRow
                title={t("activity.permissions.accessibility.title")}
                description={t("activity.permissions.accessibility.description")}
                state={permissions.accessibility}
                required
                prominent={settings.enabled && missingRequiredList.includes("accessibility")}
                onRequest={() => handleRequestPermission("accessibility")}
                onOpenSettings={() => handleOpenSettings("accessibility")}
              />
              <PermissionItemRow
                title={t("activity.permissions.screenRecording.title")}
                description={t("activity.permissions.screenRecording.description")}
                state={permissions.screenRecording}
                required
                prominent={settings.enabled && missingRequiredList.includes("screenRecording")}
                onRequest={() => handleRequestPermission("screenRecording")}
                onOpenSettings={() => handleOpenSettings("screenRecording")}
              />
              <PermissionItemRow
                title={t("activity.permissions.inputMonitoring.title")}
                description={t("activity.permissions.inputMonitoring.description")}
                state={permissions.inputMonitoring}
                required={false}
                prominent={false}
                onRequest={() => handleRequestPermission("inputMonitoring")}
                onOpenSettings={() => handleOpenSettings("inputMonitoring")}
              />
            </div>
          </div>
        </section>
      ) : null}

      <section className="settings-group" aria-labelledby="activity-exclusions-heading">
        <h2 id="activity-exclusions-heading" className="settings-group-heading">
          {t("activity.exclusions.title")}
        </h2>
        <p className="settings-group-description">{t("activity.exclusions.description")}</p>
        <div className="settings-card">
          <div className="settings-rows">
            <div className="settings-row settings-row-stack">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("activity.exclusions.apps.title")}</h3>
                <p className="settings-row-description">
                  {t("activity.exclusions.apps.description")}
                </p>
                <div style={{ display: "flex", gap: "var(--sp-2)", marginTop: "var(--sp-3)" }}>
                  <input
                    type="text"
                    className="dialog-input"
                    value={newAppName}
                    placeholder={t("activity.exclusions.apps.placeholder")}
                    aria-label={t("activity.exclusions.apps.title")}
                    onChange={(e) => setNewAppName(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        handleAddApp();
                      }
                    }}
                  />
                  <button
                    type="button"
                    className="btn btn-secondary"
                    aria-label={t("activity.exclusions.apps.addAria")}
                    onClick={handleAddApp}
                  >
                    <IconPlusMedium size={14} />
                    {t("activity.exclusions.apps.add")}
                  </button>
                </div>
              </div>
            </div>

            {settings.ignoredApps.length === 0 ? (
              <div className="settings-row settings-row-compact">
                <p className="settings-empty">{t("activity.exclusions.apps.empty")}</p>
              </div>
            ) : (
              settings.ignoredApps.map((appName) => (
                <div key={appName} className="settings-row settings-row-compact">
                  <div className="settings-row-info">
                    <span className="settings-row-title">{appName}</span>
                  </div>
                  <div className="settings-row-control">
                    <button
                      type="button"
                      className="icon-button icon-button-destructive"
                      aria-label={t("activity.exclusions.apps.removeAria", { name: appName })}
                      onClick={() => handleRemoveApp(appName)}
                    >
                      <IconTrashCan size={14} />
                    </button>
                  </div>
                </div>
              ))
            )}

            <div className="settings-row settings-row-stack" style={{ marginTop: "var(--sp-4)" }}>
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("activity.exclusions.domains.title")}</h3>
                <p className="settings-row-description">
                  {t("activity.exclusions.domains.description")}
                </p>
                <div style={{ display: "flex", gap: "var(--sp-2)", marginTop: "var(--sp-3)" }}>
                  <input
                    type="text"
                    className="dialog-input"
                    value={newDomainName}
                    placeholder={t("activity.exclusions.domains.placeholder")}
                    aria-label={t("activity.exclusions.domains.title")}
                    onChange={(e) => setNewDomainName(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        handleAddDomain();
                      }
                    }}
                  />
                  <button
                    type="button"
                    className="btn btn-secondary"
                    aria-label={t("activity.exclusions.domains.addAria")}
                    onClick={handleAddDomain}
                  >
                    <IconPlusMedium size={14} />
                    {t("activity.exclusions.domains.add")}
                  </button>
                </div>
              </div>
            </div>

            {settings.ignoredDomains.length === 0 ? (
              <div className="settings-row settings-row-compact">
                <p className="settings-empty">{t("activity.exclusions.domains.empty")}</p>
              </div>
            ) : (
              settings.ignoredDomains.map((domainName) => (
                <div key={domainName} className="settings-row settings-row-compact">
                  <div className="settings-row-info">
                    <span className="settings-row-title">{domainName}</span>
                  </div>
                  <div className="settings-row-control">
                    <button
                      type="button"
                      className="icon-button icon-button-destructive"
                      aria-label={t("activity.exclusions.domains.removeAria", {
                        domain: domainName,
                      })}
                      onClick={() => handleRemoveDomain(domainName)}
                    >
                      <IconTrashCan size={14} />
                    </button>
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      </section>

      <section className="settings-group" aria-labelledby="activity-work-hours-heading">
        <h2 id="activity-work-hours-heading" className="settings-group-heading">
          {t("activity.workHours.title")}
        </h2>
        <div className="settings-card">
          <div className="settings-rows">
            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("activity.workHours.enable")}</h3>
                <p className="settings-row-description">
                  {t("activity.workHours.enableDescription")}
                </p>
              </div>
              <div className="settings-row-control">
                <Switch
                  checked={settings.workHours.enabled}
                  disabled={saving}
                  aria-label={t("activity.workHours.enable")}
                  onCheckedChange={(enabled) =>
                    void saveSettings({
                      ...settings,
                      workHours: { ...settings.workHours, enabled },
                    })
                  }
                />
              </div>
            </div>

            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("activity.workHours.daysTitle")}</h3>
              </div>
              <div className="settings-row-control" style={{ display: "flex", gap: "var(--sp-1)" }}>
                {ISO_WEEKDAYS.map((day) => {
                  const selected = settings.workHours.days.includes(day);
                  const labelKey = `activity.workHours.day.${day}` as MessageKey;
                  return (
                    <button
                      key={day}
                      type="button"
                      className="btn btn-secondary"
                      style={{
                        minWidth: "34px",
                        padding: "0 var(--sp-2)",
                        background: selected ? "var(--secondary)" : undefined,
                        fontWeight: selected ? "var(--fw-medium)" : 400,
                        boxShadow: selected ? "0 0 0 1px var(--border-subtle)" : undefined,
                      }}
                      aria-pressed={selected}
                      disabled={saving}
                      onClick={() => handleToggleWeekday(day)}
                    >
                      {t(labelKey)}
                    </button>
                  );
                })}
              </div>
            </div>

            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("activity.workHours.start")}</h3>
              </div>
              <div className="settings-row-control">
                <input
                  type="time"
                  className="dialog-input"
                  style={{ width: "120px" }}
                  value={settings.workHours.start}
                  aria-label={t("activity.workHours.start")}
                  disabled={saving}
                  onChange={(e) =>
                    void saveSettings({
                      ...settings,
                      workHours: { ...settings.workHours, start: e.target.value },
                    })
                  }
                />
              </div>
            </div>

            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("activity.workHours.end")}</h3>
              </div>
              <div className="settings-row-control">
                <input
                  type="time"
                  className="dialog-input"
                  style={{ width: "120px" }}
                  value={settings.workHours.end}
                  aria-label={t("activity.workHours.end")}
                  disabled={saving}
                  onChange={(e) =>
                    void saveSettings({
                      ...settings,
                      workHours: { ...settings.workHours, end: e.target.value },
                    })
                  }
                />
              </div>
            </div>
          </div>
        </div>
      </section>

      <DaySummarySettingsSection
        settings={settings}
        saving={saving}
        onSave={(next) => void saveSettings(next)}
      />

      <section className="settings-group" aria-labelledby="activity-retention-heading">
        <h2 id="activity-retention-heading" className="settings-group-heading">
          {t("activity.retention.title")}
        </h2>
        <div className="settings-card">
          <div className="settings-rows">
            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("activity.retention.days")}</h3>
                <p className="settings-row-description">
                  {t("activity.retention.daysDescription")}
                </p>
              </div>
              <div className="settings-row-control">
                <input
                  type="number"
                  min={1}
                  max={365}
                  className="dialog-input"
                  style={{ width: "90px" }}
                  value={settings.retentionDays}
                  aria-label={t("activity.retention.days")}
                  disabled={saving}
                  onChange={(e) => {
                    const parsed = parseInt(e.target.value, 10);
                    if (!Number.isNaN(parsed) && parsed >= 1 && parsed <= 365) {
                      void saveSettings({ ...settings, retentionDays: parsed });
                    }
                  }}
                />
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="settings-group" aria-labelledby="activity-options-heading">
        <h2 id="activity-options-heading" className="settings-group-heading">
          {t("activity.options.title")}
        </h2>
        <div className="settings-card">
          <div className="settings-rows">
            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("activity.options.secondaryMonitors")}</h3>
                <p className="settings-row-description">
                  {t("activity.options.secondaryMonitorsDescription")}
                </p>
              </div>
              <div className="settings-row-control">
                <Switch
                  checked={settings.secondaryMonitors}
                  disabled={saving}
                  aria-label={t("activity.options.secondaryMonitors")}
                  onCheckedChange={(secondaryMonitors) =>
                    void saveSettings({ ...settings, secondaryMonitors })
                  }
                />
              </div>
            </div>

            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">
                  {t("activity.options.pauseOnProtectedVideo")}
                </h3>
                <p className="settings-row-description">
                  {t("activity.options.pauseOnProtectedVideoDescription")}
                </p>
              </div>
              <div className="settings-row-control">
                <Switch
                  checked={settings.pauseOnProtectedVideo}
                  disabled={saving}
                  aria-label={t("activity.options.pauseOnProtectedVideo")}
                  onCheckedChange={(pauseOnProtectedVideo) =>
                    void saveSettings({ ...settings, pauseOnProtectedVideo })
                  }
                />
              </div>
            </div>

            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("activity.options.inputEvents")}</h3>
                <p className="settings-row-description">
                  {t("activity.options.inputEventsDescription")}
                </p>
              </div>
              <div className="settings-row-control">
                <Switch
                  checked={settings.inputEvents}
                  disabled={saving}
                  aria-label={t("activity.options.inputEvents")}
                  onCheckedChange={(inputEvents) => void saveSettings({ ...settings, inputEvents })}
                />
              </div>
            </div>
          </div>
        </div>
      </section>

      <CodingAgentSourcesSection
        sources={settings.codingAgents}
        saving={saving}
        onChange={(codingAgents) => void saveSettings({ ...settings, codingAgents })}
      />

      {status.debugExportAvailable ? (
        <section className="settings-group" aria-labelledby="activity-debug-heading">
          <h2 id="activity-debug-heading" className="settings-group-heading">
            {t("activity.debug.title")}
          </h2>
          <div className="settings-card">
            <div className="settings-rows">
              <div className="settings-row">
                <div className="settings-row-info">
                  <h3 className="settings-row-title">{t("activity.debug.export")}</h3>
                  <p className="settings-row-description">
                    {t("activity.debug.exportDescription")}
                  </p>
                </div>
                <div className="settings-row-control">
                  <button
                    type="button"
                    className="btn btn-secondary"
                    disabled={debugExporting}
                    onClick={handleDebugExport}
                  >
                    {t("activity.debug.exportButton")}
                  </button>
                </div>
              </div>
              {debugResult ? (
                <div className="settings-row">
                  <div className="settings-row-info">
                    <p className="settings-status" role="status">
                      {t("activity.debug.exportResult", {
                        path: debugResult.path,
                        frames: debugResult.frames,
                        secondaryFrames: debugResult.secondaryFrames,
                        inputEvents: debugResult.inputEvents,
                        pauses: debugResult.pauses,
                      })}
                    </p>
                  </div>
                </div>
              ) : null}
            </div>
          </div>
        </section>
      ) : null}
    </>
  );
}

function PermissionItemRow({
  title,
  description,
  state,
  required,
  prominent,
  onRequest,
  onOpenSettings,
}: {
  title: string;
  description: string;
  state: ActivityPermissionState;
  required: boolean;
  prominent: boolean;
  onRequest: () => void;
  onOpenSettings: () => void;
}) {
  const t = useT();
  const isGranted = state === "granted";
  const tone = isGranted ? "allowed" : required ? "attention" : "unknown";

  const statusLabel = isGranted
    ? t("activity.permissions.status.granted")
    : required
      ? t("activity.permissions.status.missing")
      : t("activity.permissions.status.optionalMissing");

  return (
    <div
      className="settings-row"
      style={
        prominent
          ? {
              outline: "1px solid color-mix(in oklch, var(--warm-strong) 40%, transparent)",
              borderRadius: "var(--r-md)",
              padding: "var(--sp-3) var(--sp-4)",
            }
          : undefined
      }
    >
      <div className="settings-row-info">
        <h3 className="settings-row-title">{title}</h3>
        <p className="settings-row-description">{description}</p>
      </div>
      <div className="settings-row-control settings-permission-control">
        <span
          className="settings-permission-status"
          data-status={tone}
          role="img"
          aria-label={statusLabel}
          title={statusLabel}
        >
          {isGranted ? (
            <IconCircleCheck size={16} />
          ) : required ? (
            <IconExclamationCircle size={16} />
          ) : (
            <IconCircleQuestionmark size={16} />
          )}
        </span>
        {!isGranted ? (
          <>
            <button
              type="button"
              className="btn btn-secondary"
              aria-label={t("activity.permissions.requestAria", { permission: title })}
              onClick={onRequest}
            >
              {t("activity.permissions.request")}
            </button>
            <button
              type="button"
              className="btn btn-secondary"
              aria-label={t("activity.permissions.openSettingsAria", { permission: title })}
              onClick={onOpenSettings}
            >
              {t("activity.permissions.openSettings")}
            </button>
          </>
        ) : null}
      </div>
    </div>
  );
}
