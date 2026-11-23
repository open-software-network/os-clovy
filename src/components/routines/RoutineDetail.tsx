import { IconDotGrid1x3Horizontal } from "central-icons/IconDotGrid1x3Horizontal";
import { IconArrowRotateClockwise } from "central-icons/IconArrowRotateClockwise";
import { IconCalendarRepeat } from "central-icons/IconCalendarRepeat";
import { IconPlay } from "central-icons/IconPlay";
import { IconShieldCrossed } from "central-icons/IconShieldCrossed";
import { IconTrashCan } from "central-icons/IconTrashCan";
import { IconPause } from "central-icons/IconPause";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import {
  TRIGGER_META,
  eventTriggerScheduleDraft,
  routineToolsetsFor,
  triggerConfigFromDraft,
  triggerScopeWarning,
  type TriggerDraft,
} from "../../lib/connectors";
import { useConnectorPolicy } from "../../lib/connector-policy";
import { useExperimentalFlags } from "../../lib/experimental-flags";
import {
  pauseRoutine,
  resumeRoutine,
  routineUnrestricted,
  type RoutineJob,
  type RoutineUpdates,
} from "../../lib/agent-routines";
import type { RoutineRunSession } from "../../lib/agent-routine-history";
import {
  compactScheduleLabel,
  draftFromSchedule,
  scheduleFromDraft,
  type ScheduleDraft,
} from "../../lib/routine-schedule";
import {
  connectorTriggerDelete,
  connectorTriggerSet,
  connectorTriggersList,
  connectorsList,
  routineTrustGet,
  routineTrustSet,
  routineBrowserAccessGet,
  routineBrowserAccessSet,
  type ConnectorAccount,
  type ConnectorTrigger,
  type RoutineTrust,
  type RoutineTrustMode,
  type RoutineBrowserAccess,
} from "../../lib/tauri";
import { messageFromError } from "../../lib/errors";
import { useT } from "../../i18n";
import { BreadcrumbBar } from "../ui/BreadcrumbBar";
import { HoverTip } from "../ui/HoverTip";
import { Switch } from "../ui/Switch";
import { toast } from "../ui/Toaster";
import { userFacingFailureMessage } from "../note-editor/NoteFailureBanner";
import { GrowingTextarea } from "./GrowingTextarea";
import { RoutineModePicker } from "./RoutineModePicker";
import { formatRunTime, RoutineRunList } from "./RoutineRunList";
import { TriggerPicker } from "./TriggerPicker";
import { TrustModePicker } from "./TrustModePicker";

/** Maps a stored connector trigger back onto the editor's "When" model. Key
 * order matters: the dirty check compares drafts via JSON.stringify, so this
 * must build objects in the same shape TriggerPicker emits. */
function triggerDraftFromStored(stored: ConnectorTrigger): TriggerDraft {
  if (stored.kind === "email_received") return { source: "email_received" };
  const lead = stored.config.leadMinutes;
  const external = stored.config.externalOnly;
  return {
    source: "event_upcoming",
    leadMinutes: typeof lead === "number" ? lead : 30,
    externalOnly: typeof external === "boolean" ? external : true,
  };
}

type RoutineDetailProps = {
  routine: RoutineJob;
  initialTab?: "details" | "history";
  /** Past runs of this routine only. */
  runs: RoutineRunSession[];
  busy: boolean;
  saving: boolean;
  error: string | null;
  onBack: () => void;
  onSave: (updates: RoutineUpdates) => Promise<void>;
  onToggleActive: () => void;
  onRunNow: () => Promise<void>;
  onDelete: () => void;
  onOpenRun: (run: RoutineRunSession) => void;
  onRetryLoad?: () => void;
  retrying?: boolean;
};

/** One routine, fully editable in place: schedule, instructions, access and
 * name save through Clovy's routine commands, while activity (toggle, run now,
 * run history) acts immediately. Mount with `key={routine.job_id}` — the
 * draft fields initialize from the routine once and reconcile through the
 * dirty comparison after saves refresh the prop. */
export function RoutineDetail({
  routine,
  initialTab = "details",
  runs,
  busy,
  saving,
  error,
  onBack,
  onSave,
  onToggleActive,
  onRunNow,
  onDelete,
  onOpenRun,
  onRetryLoad,
  retrying,
}: RoutineDetailProps) {
  const t = useT();
  const { browserUseEnabled } = useExperimentalFlags();
  const { policy } = useConnectorPolicy();
  const [name, setName] = useState(routine.name);
  const [draft, setDraft] = useState<ScheduleDraft>(() => draftFromSchedule(routine.schedule));
  const [prompt, setPrompt] = useState(routine.prompt);
  const [unrestricted, setUnrestricted] = useState(() => routineUnrestricted(routine));
  const [storedBrowserAccess, setStoredBrowserAccess] = useState<RoutineBrowserAccess | null>(null);
  const [browserAccess, setBrowserAccess] = useState(false);
  // Connector trust + trigger. All loads degrade quietly: a routine without
  // a trust record (or a build without the connectors module) reads as
  // read only with no trigger, which is exactly the stored default.
  const [storedTrust, setStoredTrust] = useState<RoutineTrust | null>(null);
  const [trustMode, setTrustMode] = useState<RoutineTrustMode>("read_only");
  const [autonomousTools, setAutonomousTools] = useState<string[]>([]);
  const [storedTrigger, setStoredTrigger] = useState<ConnectorTrigger | null>(null);
  const [trigger, setTrigger] = useState<TriggerDraft>({ source: "schedule" });
  const [accounts, setAccounts] = useState<ConnectorAccount[]>([]);
  const [activeTab, setActiveTab] = useState<"details" | "history">(initialTab);
  // "Run now" only queues the job for the scheduler's next tick, so the
  // confirmation is a short-lived label swap rather than a new run row.
  const [queued, setQueued] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [tabIndicator, setTabIndicator] = useState({ x: 0, width: 0 });
  const queueTimer = useRef<number>();
  const menuWrapRef = useRef<HTMLDivElement>(null);
  const detailsTabRef = useRef<HTMLButtonElement>(null);
  const historyTabRef = useRef<HTMLButtonElement>(null);
  const previousRoutineNameRef = useRef(routine.name);
  useEffect(() => () => window.clearTimeout(queueTimer.current), []);

  useEffect(() => {
    if (!menuOpen) return;
    function onPointer(event: MouseEvent) {
      if (!menuWrapRef.current?.contains(event.target as Node)) {
        setMenuOpen(false);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setMenuOpen(false);
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  useEffect(() => {
    let cancelled = false;
    routineTrustGet(routine.job_id)
      .then((trust) => {
        if (cancelled || !trust) return;
        setStoredTrust(trust);
        setTrustMode(trust.trustMode);
        setAutonomousTools(trust.autonomousTools);
      })
      .catch(() => {});
    routineBrowserAccessGet(routine.job_id)
      .then((access) => {
        if (cancelled) return;
        setStoredBrowserAccess(access);
        setBrowserAccess(access.enabled);
      })
      .catch(() => {});
    connectorTriggersList(routine.job_id)
      .then((triggers) => {
        if (cancelled) return;
        const stored = triggers[0] ?? null;
        setStoredTrigger(stored);
        if (stored) setTrigger(triggerDraftFromStored(stored));
      })
      .catch(() => {});
    connectorsList()
      .then((list) => {
        if (!cancelled) setAccounts(list);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [routine.job_id]);

  useEffect(() => {
    const previousName = previousRoutineNameRef.current;
    previousRoutineNameRef.current = routine.name;
    setName((current) => {
      const trimmed = current.trim();
      if (!trimmed || trimmed === previousName) return routine.name;
      return current;
    });
  }, [routine]);

  useLayoutEffect(() => {
    function updateIndicator() {
      const tab = activeTab === "details" ? detailsTabRef.current : historyTabRef.current;
      if (!tab) return;
      setTabIndicator({ x: tab.offsetLeft, width: tab.offsetWidth });
    }
    updateIndicator();
    window.addEventListener("resize", updateIndicator);
    return () => window.removeEventListener("resize", updateIndicator);
  }, [activeTab]);

  const paused = routine.state === "paused";
  const completed = routine.state === "completed";

  const nameChanged = name.trim().length > 0 && name.trim() !== routine.name;
  const scheduleChanged =
    trigger.source === "schedule" &&
    JSON.stringify(draft) !== JSON.stringify(draftFromSchedule(routine.schedule));
  const promptChanged = prompt !== routine.prompt;
  const modeChanged = unrestricted !== routineUnrestricted(routine);
  const browserAccessChanged =
    storedBrowserAccess !== null && browserAccess !== storedBrowserAccess.enabled;
  const storedTrustMode = storedTrust?.trustMode ?? "read_only";
  const storedTools = storedTrust?.autonomousTools ?? [];
  const trustChanged =
    trustMode !== storedTrustMode ||
    (trustMode === "autonomous" && JSON.stringify(autonomousTools) !== JSON.stringify(storedTools));
  const storedTriggerDraft: TriggerDraft = storedTrigger
    ? triggerDraftFromStored(storedTrigger)
    : { source: "schedule" };
  const triggerChanged = JSON.stringify(trigger) !== JSON.stringify(storedTriggerDraft);
  const dirty =
    nameChanged ||
    scheduleChanged ||
    promptChanged ||
    modeChanged ||
    browserAccessChanged ||
    trustChanged ||
    triggerChanged;

  function changeTrigger(next: TriggerDraft) {
    // Event routines store a far-future one-off schedule only as a dormant
    // legacy placeholder. Do not expose or resume that placeholder when the
    // user switches back to scheduling; start from a useful daily default.
    if (
      trigger.source !== "schedule" &&
      next.source === "schedule" &&
      scheduleFromDraft(draft) === eventTriggerScheduleDraft().schedule
    ) {
      setDraft({ kind: "daily", time: "09:00" });
    }
    setTrigger(next);
  }

  async function save() {
    const connectorPolicyNeeded =
      trustChanged ||
      triggerChanged ||
      browserAccessChanged ||
      (modeChanged && storedTrust != null);
    if (connectorPolicyNeeded && !policy) {
      toast.error(t("routines.policyLoading"));
      return;
    }
    const updates: RoutineUpdates = {};
    if (nameChanged) updates.name = name.trim();
    if (scheduleChanged) updates.schedule = scheduleFromDraft(draft);
    if (promptChanged) updates.prompt = prompt;

    // Validate a new event trigger up front, before any persistent write. An
    // account/scope problem must abort the save before trust is minted or the
    // cron job is touched, so a validation failure never leaves a half-applied
    // trust/grant change to unwind.
    let triggerAccount: ConnectorAccount | undefined;
    const switchingToEvent = triggerChanged && trigger.source !== "schedule";
    if (switchingToEvent) {
      // Google only: triggers poll Gmail/Calendar, so a connected Linear
      // workspace must never be picked as the trigger account.
      triggerAccount = accounts.find(
        (entry) => entry.provider === "google" && entry.status === "connected",
      );
      if (!triggerAccount) {
        toast.error(t("routines.detail.connectGoogleFirst"));
        return;
      }
      // The account must hold the scope this trigger's daemon polls, or the
      // routine saves but never fires (the Gmail/calendar call fails).
      const scopeIssue = policy
        ? triggerScopeWarning(policy, trigger, triggerAccount.scopes)
        : t("routines.policyLoading");
      if (scopeIssue) {
        toast.error(scopeIssue);
        return;
      }
    }

    // The trust record before this save, so a failed cron-job update can roll
    // the trust/grant change back and keep the two in sync.
    const previousTrust = storedTrust;
    const previousBrowserAccess = storedBrowserAccess;

    let nextBrowserAccess = storedBrowserAccess;
    if (browserAccessChanged) {
      try {
        nextBrowserAccess = await routineBrowserAccessSet({
          jobId: routine.job_id,
          enabled: browserAccess,
        });
        setStoredBrowserAccess(nextBrowserAccess);
      } catch (err) {
        toast.error(messageFromError(err));
        return;
      }
    }

    // Trust first: an autonomous grant mints per-job auto server names that
    // the toolset override must reference.
    if (trustChanged) {
      if (!policy) return;
      try {
        const stored = await routineTrustSet({
          jobId: routine.job_id,
          trustMode,
          autonomousTools: trustMode === "autonomous" ? autonomousTools : undefined,
        });
        setStoredTrust(stored);
        updates.enabledToolsets = routineToolsetsFor(policy, trustMode, {
          unrestricted,
          autonomousServers: stored.autonomousServers,
          routineBrowserServer: nextBrowserAccess?.serverName ?? undefined,
        });
      } catch (err) {
        if (browserAccessChanged && previousBrowserAccess) {
          try {
            const restored = await routineBrowserAccessSet({
              jobId: routine.job_id,
              enabled: previousBrowserAccess.enabled,
            });
            setStoredBrowserAccess(restored);
          } catch (restoreError) {
            toast.error(messageFromError(restoreError));
          }
        }
        toast.error(messageFromError(err));
        return;
      }
    } else if (modeChanged || browserAccessChanged) {
      if (!policy && (storedTrust || nextBrowserAccess?.enabled)) return;
      // No trust change: connector routines recompose their toolsets around
      // the new base; plain routines keep the legacy boolean path.
      if (storedTrust) {
        if (!policy) return;
        updates.enabledToolsets = routineToolsetsFor(policy, trustMode, {
          unrestricted,
          autonomousServers: storedTrust.autonomousServers,
          routineBrowserServer: nextBrowserAccess?.serverName ?? undefined,
        });
      } else if (nextBrowserAccess?.enabled && nextBrowserAccess.serverName) {
        if (!policy) return;
        updates.enabledToolsets = routineToolsetsFor(policy, "read_only", {
          unrestricted,
          routineBrowserServer: nextBrowserAccess.serverName,
        });
      } else {
        updates.unrestricted = unrestricted;
      }
    }

    // Fold the trigger's schedule change into the routine update so it lands
    // atomically with the rest. The trigger row write and activation changes are
    // deferred until after onSave succeeds (below).
    const switchingToSchedule = triggerChanged && trigger.source === "schedule";
    const switchingFromSchedule = switchingToEvent && storedTriggerDraft.source === "schedule";
    if (switchingToSchedule) {
      // Back on a real schedule: replace the far-future placeholder so the
      // scheduler owns the job again once it is resumed below.
      updates.schedule = scheduleFromDraft(draft);
    } else if (switchingFromSchedule) {
      // Newly event-driven: park the cron schedule far in the future; the
      // trigger daemon fires the job once its trigger row is written below.
      updates.schedule = eventTriggerScheduleDraft().schedule;
    }

    try {
      await onSave(updates);
    } catch {
      // The cron job update failed (onSave rejects and has already surfaced the
      // error) after trust was persisted and grants minted or deleted, so the
      // routine's DB trust and the job's toolsets now disagree. Roll the trust
      // change back to keep them consistent: otherwise a downgrade to read only
      // would have deleted the grant while the job kept its autonomous toolsets,
      // and gate_action parks (not denies) an orphaned grant, so an approval
      // could still let that run act on Google. No trigger row was written yet,
      // so there is nothing else to unwind.
      if (trustChanged) {
        try {
          const restored = await routineTrustSet({
            jobId: routine.job_id,
            trustMode: previousTrust?.trustMode ?? "read_only",
            autonomousTools:
              previousTrust?.trustMode === "autonomous" ? previousTrust.autonomousTools : undefined,
          });
          setStoredTrust(restored);
        } catch (err) {
          toast.error(messageFromError(err));
        }
      }
      if (browserAccessChanged && previousBrowserAccess) {
        try {
          const restored = await routineBrowserAccessSet({
            jobId: routine.job_id,
            enabled: previousBrowserAccess.enabled,
          });
          setStoredBrowserAccess(restored);
        } catch (err) {
          toast.error(messageFromError(err));
        }
      }
      return;
    }

    // Trigger side effects run only after the cron save succeeded, so a failed
    // onSave can never leave a trigger firing a routine whose schedule/toolsets
    // never saved. If a side effect here fails the routine stays dormant (the
    // far-future schedule and active state already persisted), which is
    // safe: it under-fires rather than acting on stale config. Surface it so the
    // user can retry.
    if (triggerChanged) {
      try {
        if (trigger.source === "schedule") {
          // Ensure the routine is active first so a runtime failure leaves the existing event
          // trigger untouched. Only remove that fallback after the scheduled
          // job is live again.
          await resumeRoutine(routine.job_id);
          try {
            if (storedTrigger) {
              await connectorTriggerDelete(storedTrigger.id);
            }
          } catch (deleteError) {
            // The trigger still exists. Restore the previous inactive state so
            // schedule and event sources cannot both fire this routine.
            try {
              await pauseRoutine(routine.job_id);
            } catch (restoreError) {
              toast.error(
                t("routines.detail.restoreFailed", { error: messageFromError(restoreError) }),
              );
            }
            throw deleteError;
          }
          setStoredTrigger(null);
        } else if (triggerAccount) {
          const stored = await connectorTriggerSet({
            jobId: routine.job_id,
            kind: trigger.source,
            accountId: triggerAccount.accountId,
            config: triggerConfigFromDraft(trigger),
          });
          setStoredTrigger(stored);
        }
      } catch (err) {
        toast.error(messageFromError(err));
        return;
      }
    }
  }

  async function runNow() {
    await onRunNow();
    setQueued(true);
    window.clearTimeout(queueTimer.current);
    queueTimer.current = window.setTimeout(() => setQueued(false), 5000);
  }

  const storedFailure =
    routine.last_status === "error"
      ? routine.last_error || routine.last_delivery_error || undefined
      : undefined;
  const failure = storedFailure ? userFacingFailureMessage(storedFailure) : null;

  return (
    <section className="routine-detail" aria-label={routine.name}>
      <BreadcrumbBar
        backLabel={t("routines.backToRoutines")}
        onBack={onBack}
        items={[
          { label: t("routines.title"), onClick: onBack },
          { label: name.trim() || routine.name },
        ]}
        actions={
          <div className="routine-detail-actions">
            <div className="agent-session-menu-wrap" ref={menuWrapRef}>
              <button
                type="button"
                className="btn btn-ghost routine-detail-menu-trigger"
                aria-label={t("routines.detail.actions")}
                aria-haspopup="menu"
                aria-expanded={menuOpen}
                onClick={() => setMenuOpen((open) => !open)}
              >
                <IconDotGrid1x3Horizontal size={16} />
              </button>
              {menuOpen ? (
                <div className="sidebar-identity-menu agent-session-menu" role="menu">
                  <button
                    type="button"
                    role="menuitem"
                    className="destructive"
                    disabled={busy}
                    onClick={() => {
                      setMenuOpen(false);
                      onDelete();
                    }}
                  >
                    <IconTrashCan size={14} />
                    {t("routines.deleteRoutine")}
                  </button>
                </div>
              ) : null}
            </div>
            <button
              type="button"
              className="btn btn-secondary"
              disabled={busy || queued || routine.state !== "scheduled"}
              onClick={() => void runNow()}
            >
              <IconPlay size={13} aria-hidden />
              {queued ? t("routines.detail.queued") : t("routines.runNow")}
            </button>
            <button
              type="button"
              className="primary-action primary-solid"
              disabled={!dirty || !prompt.trim() || saving || busy}
              onClick={() => void save()}
            >
              {saving ? t("routines.detail.saving") : t("common.save")}
            </button>
          </div>
        }
      />

      <div className="routine-detail-content">
        <div className="routine-detail-title-row">
          <input
            className="routine-detail-name"
            value={name}
            placeholder={t("routines.routineName")}
            aria-label={t("routines.routineName")}
            onChange={(event) => setName(event.currentTarget.value)}
          />
          {!completed ? (
            <div
              className="routine-detail-active-control"
              data-state={routine.state === "scheduled" ? "active" : "paused"}
            >
              <Switch
                checked={routine.state === "scheduled"}
                disabled={busy}
                aria-label={t("routines.detail.activeToggle", {
                  name: name.trim() || routine.name,
                })}
                onCheckedChange={onToggleActive}
              />
            </div>
          ) : null}
        </div>

        <div className="routine-detail-meta">
          {routine.state === "scheduled" ? (
            <span className="routine-meta-pill routine-meta-pill-warm">
              {t("routines.detail.active")}
            </span>
          ) : null}
          {routineUnrestricted(routine) ? (
            <HoverTip
              tip={t("routines.unrestrictedTip")}
              className="routine-meta-pill routine-meta-pill-warm"
              tabIndex={0}
            >
              <IconShieldCrossed size={11} aria-hidden />
              {t("routines.mode.unrestricted")}
            </HoverTip>
          ) : null}
          {paused ? (
            <span className="routine-meta-pill">
              <IconPause size={12} aria-hidden />
              {t("routines.paused")}
            </span>
          ) : null}
          {completed ? <span className="routine-meta-pill">{t("routines.completed")}</span> : null}
          <span className="routine-meta-pill">
            <IconCalendarRepeat size={12} aria-hidden />
            {storedTrigger
              ? TRIGGER_META[storedTrigger.kind].label
              : compactScheduleLabel(routine.schedule)}
          </span>
          {completed && routine.last_run_at ? (
            <span className="routine-meta-pill">
              {t("routines.lastRan", { time: formatRunTime(routine.last_run_at) })}
            </span>
          ) : null}
        </div>

        {error ? (
          <div className="error-banner routines-error-banner" role="alert">
            <p>{error}</p>
            {onRetryLoad ? (
              <button
                type="button"
                className="btn btn-ghost"
                onClick={onRetryLoad}
                disabled={retrying}
                aria-busy={retrying || undefined}
              >
                <IconArrowRotateClockwise size={14} className="balance-refresh-icon" aria-hidden />
                {t("common.tryAgain")}
              </button>
            ) : null}
          </div>
        ) : null}
        {failure ? (
          <div className="routine-detail-failure" role="status">
            <strong>{t("routines.detail.lastRunFailed")}</strong> {failure}
          </div>
        ) : null}

        <div
          className="routine-detail-tabs"
          style={
            {
              "--routine-tabs-indicator-x": `${tabIndicator.x}px`,
              "--routine-tabs-indicator-w": `${tabIndicator.width}px`,
            } as CSSProperties
          }
          role="tablist"
          aria-label={t("routines.detail.sections")}
        >
          <button
            ref={detailsTabRef}
            id="routine-details-tab"
            type="button"
            role="tab"
            aria-selected={activeTab === "details"}
            aria-controls="routine-details-panel"
            onClick={() => setActiveTab("details")}
          >
            {t("routines.detail.details")}
          </button>
          <button
            ref={historyTabRef}
            id="routine-history-tab"
            type="button"
            role="tab"
            aria-selected={activeTab === "history"}
            aria-controls="routine-history-panel"
            onClick={() => setActiveTab("history")}
          >
            {t("routines.runHistory")}
          </button>
        </div>

        {activeTab === "details" ? (
          <div
            id="routine-details-panel"
            className="routine-detail-body"
            role="tabpanel"
            aria-labelledby="routine-details-tab"
          >
            <section className="settings-group" aria-labelledby="routine-schedule">
              <h2 id="routine-schedule" className="settings-group-heading">
                {t("routines.section.when")}
              </h2>
              <div className="settings-card">
                <TriggerPicker
                  trigger={trigger}
                  scheduleDraft={draft}
                  hasAccount={accounts.some(
                    (entry) => entry.provider === "google" && entry.status === "connected",
                  )}
                  scopeWarning={
                    policy
                      ? triggerScopeWarning(
                          policy,
                          trigger,
                          accounts.find(
                            (entry) => entry.provider === "google" && entry.status === "connected",
                          )?.scopes ?? null,
                        )
                      : null
                  }
                  onTriggerChange={changeTrigger}
                  onScheduleChange={setDraft}
                />
              </div>
            </section>

            <section className="settings-group" aria-labelledby="routine-instructions">
              <h2 id="routine-instructions" className="settings-group-heading">
                {t("routines.section.instructions")}
              </h2>
              <GrowingTextarea
                className="routine-detail-instructions"
                value={prompt}
                aria-label={t("routines.section.instructions")}
                onChange={(event) => setPrompt(event.currentTarget.value)}
              />
            </section>

            <section className="settings-group" aria-labelledby="routine-access">
              <h2 id="routine-access" className="settings-group-heading">
                {t("routines.section.access")}
              </h2>
              <div className="settings-card">
                <RoutineModePicker unrestricted={unrestricted} onChange={setUnrestricted} />
                {browserUseEnabled ? (
                  <div className="settings-row">
                    <div className="settings-row-info">
                      <div className="settings-row-title">{t("routines.detail.browserUse")}</div>
                      <div className="settings-row-description">
                        {t("routines.detail.browserUseDescription")}
                      </div>
                    </div>
                    <div className="settings-row-control">
                      <Switch
                        checked={browserAccess}
                        disabled={storedBrowserAccess === null}
                        aria-label={t("routines.detail.browserUseToggle")}
                        onCheckedChange={setBrowserAccess}
                      />
                    </div>
                  </div>
                ) : null}
                {routine.script ? (
                  <p className="routine-detail-script-note">
                    {t("routines.detail.scriptNote", { script: routine.script })}
                  </p>
                ) : null}
              </div>
            </section>

            <section className="settings-group" aria-labelledby="routine-trust">
              <h2 id="routine-trust" className="settings-group-heading">
                {t("routines.section.actions")}
              </h2>
              <div className="settings-card">
                <TrustModePicker
                  policy={policy}
                  value={trustMode}
                  runCount={storedTrust?.approvalRunCount ?? 0}
                  autonomousTools={autonomousTools}
                  onChange={setTrustMode}
                  onAutonomousToolsChange={setAutonomousTools}
                />
              </div>
            </section>
          </div>
        ) : (
          <section
            id="routine-history-panel"
            className="routine-detail-body"
            role="tabpanel"
            aria-labelledby="routine-history-tab"
            aria-label={t("routines.runHistory")}
          >
            {runs.length > 0 ? (
              <div className="settings-card routines-runs-card">
                <RoutineRunList runs={runs} label={() => routine.name} onOpen={onOpenRun} />
              </div>
            ) : (
              <p className="routines-runs-empty">{t("routines.detail.noRuns")}</p>
            )}
          </section>
        )}
      </div>
    </section>
  );
}
