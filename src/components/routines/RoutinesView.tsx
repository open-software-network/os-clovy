import { listen } from "@tauri-apps/api/event";
import { IconArrowRotateClockwise } from "central-icons/IconArrowRotateClockwise";
import { IconCalendarRepeat } from "central-icons/IconCalendarRepeat";
import { IconDotGrid1x3Horizontal } from "central-icons/IconDotGrid1x3Horizontal";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { IconPencil } from "central-icons/IconPencil";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { IconPlay } from "central-icons/IconPlay";
import { IconShieldCrossed } from "central-icons/IconShieldCrossed";
import { IconTrashCan } from "central-icons/IconTrashCan";
import { IconZap } from "central-icons/IconZap";
import { IconPause } from "central-icons/IconPause";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { messageFromError } from "../../lib/errors";
import { useClovyDataChanged } from "../../lib/clovy-data-events";
import type { AgentRuntimeEvent } from "../../lib/agent-runtime-contract";
import {
  TRUST_MODE_META,
  eventTriggerScheduleDraft,
  isCreditableRun,
  routineToolsetsFor,
  routineTrustModeFromToolsets,
  triggerConfigFromDraft,
} from "../../lib/connectors";
import { useConnectorPolicy } from "../../lib/connector-policy";
import {
  isReplaceableRoutineRunTitle,
  listRoutineRunSessions,
  onRoutineRunOpen,
  routineIdFromSession,
  type RoutineRunSession,
} from "../../lib/agent-routine-history";
import {
  createRoutine,
  listRoutines,
  pauseRoutine,
  removeRoutine,
  resumeRoutine,
  routineUnrestricted,
  triggerRoutine,
  UNRESTRICTED_ROUTINE_TOOLSETS,
  updateRoutine,
  type RoutineJob,
  type RoutineUpdates,
} from "../../lib/agent-routines";
import { compactScheduleLabel, humanizeSchedule } from "../../lib/routine-schedule";
import { useForcedEmptyStates } from "../../lib/empty-states-demo";
import {
  connectorTriggerSet,
  routineTrustRecordRun,
  routineTrustSet,
  type ConnectorPolicyCatalog,
} from "../../lib/tauri";
import { t as translate, useT } from "../../i18n";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { HoverTip } from "../ui/HoverTip";
import { RoutineCreate, type RoutineCreateInput } from "./RoutineCreate";
import { RoutineDetail } from "./RoutineDetail";
import { formatRunTime, RoutineRunList } from "./RoutineRunList";
import { RoutineRunResult } from "./RoutineRunResult";
import { ROUTINE_TEMPLATES, type RoutineTemplate } from "./routine-templates";

const NO_ROUTINES: RoutineJob[] = [];
const NO_RUNS: RoutineRunSession[] = [];
const RUN_HISTORY_REFRESH_MS = 10000;
// Must match the native event name in src-tauri/src/agent_runtime/host.rs.
const AGENT_RUNTIME_EVENT = "clovy://agent-runtime-event";

/**
 * Advances the earned-autonomy counter by reporting each finished run to the
 * local store, which credits it exactly once and only when the routine is in
 * approval mode with the run finishing after approval was enabled. Reporting
 * every finished run (rather than seeding a client-side baseline) is what lets
 * background runs that completed while this view was closed still count on the
 * next visit. Best-effort: a failure just retries on the next refresh.
 *
 * `reported` is a per-mount chatter guard so the 10s refresh does not re-report
 * the same run repeatedly; the local store is the durable, idempotent ledger, so a
 * fresh mount re-reporting a run is harmless.
 */
async function creditApprovalRuns(runs: RoutineRunSession[], reported: Set<string>): Promise<void> {
  for (const run of runs.filter(isCreditableRun)) {
    if (reported.has(run.id)) continue;
    const jobId = routineIdFromSession(run);
    const runEndedAt = run.ended_at ?? run.endedAt ?? null;
    if (!jobId || !runEndedAt) continue;
    reported.add(run.id);
    try {
      await routineTrustRecordRun({ jobId, runId: run.id, runEndedAt });
    } catch {
      // Let a transient failure retry on the next refresh.
      reported.delete(run.id);
    }
  }
}

type RoutinePage = { kind: "detail"; jobId: string; tab?: "history" };

type Page =
  | { kind: "list" }
  | { kind: "create"; template?: RoutineTemplate }
  | RoutinePage
  | { kind: "result"; runId: string; back: { kind: "list" } | RoutinePage };

export function RoutinesView() {
  const t = useT();
  const { policy } = useConnectorPolicy();
  const [allRoutines, setRoutines] = useState<RoutineJob[]>([]);
  const [loadingState, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [errorRetryable, setErrorRetryable] = useState(false);
  const [query, setQuery] = useState("");
  const [busyIds, setBusyIds] = useState<ReadonlySet<string>>(new Set());
  const [pendingDelete, setPendingDelete] = useState<RoutineJob | null>(null);
  const [page, setPage] = useState<Page>({ kind: "list" });
  const [requestedRunId, setRequestedRunId] = useState<string>();
  const returnFocusRunId = useRef<string>();
  const listHeading = useRef<HTMLHeadingElement>(null);
  useEffect(() => onRoutineRunOpen(setRequestedRunId), []);
  useEffect(() => {
    if (page.kind === "result" || !returnFocusRunId.current) return;
    const target =
      document.getElementById(`routine-run-${returnFocusRunId.current}`) ??
      document.getElementById("routine-history-tab") ??
      listHeading.current;
    target?.focus();
    returnFocusRunId.current = undefined;
  }, [page.kind]);
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [detailErrorRetryable, setDetailErrorRetryable] = useState(false);
  const [refreshSpins, setRefreshSpins] = useState(0);
  const [allRuns, setRuns] = useState<RoutineRunSession[]>([]);
  const [runsUnavailableState, setRunsUnavailable] = useState(false);
  const [runsLoaded, setRunsLoaded] = useState(false);
  useEffect(() => {
    if (!requestedRunId || !runsLoaded) return;
    setPage({ kind: "result", runId: requestedRunId, back: { kind: "list" } });
    setRequestedRunId(undefined);
  }, [requestedRunId, runsLoaded]);
  const routineLoadSequenceRef = useRef(0);
  const visibleRoutineLoadSequenceRef = useRef(0);
  const visibleRoutineLoadsInFlightRef = useRef(0);
  const runLoadSequenceRef = useRef(0);
  // Run ids already reported for crediting this mount; the local store is the
  // durable idempotent ledger, this just avoids re-reporting on every refresh.
  const reportedRunsRef = useRef<Set<string>>(new Set());

  // __emptyStates() preview (dev console): render the page as a fresh
  // install would see it, real data untouched underneath.
  const forcedEmpty = useForcedEmptyStates();
  const routines = forcedEmpty ? NO_ROUTINES : allRoutines;
  const runs = forcedEmpty ? NO_RUNS : allRuns;
  const loading = !forcedEmpty && loadingState;
  const runsUnavailable = !forcedEmpty && runsUnavailableState;

  // `loading` gates the whole list and only covers the first fetch;
  // `refreshing` covers every fetch so reloads keep the list visible while
  // still signalling progress on the refresh control.
  const loadRoutines = useCallback(async (options: { silent?: boolean } = {}) => {
    const silent = options.silent ?? false;
    // A timer or terminal event is best-effort. Let an initial, manual, or
    // mutation-triggered load retain foreground result and error semantics.
    if (silent && visibleRoutineLoadsInFlightRef.current > 0) return null;
    const sequence = routineLoadSequenceRef.current + 1;
    routineLoadSequenceRef.current = sequence;
    const visibleSequence = silent ? null : visibleRoutineLoadSequenceRef.current + 1;
    if (visibleSequence !== null) {
      visibleRoutineLoadSequenceRef.current = visibleSequence;
      visibleRoutineLoadsInFlightRef.current += 1;
      setRefreshing(true);
    }
    try {
      const jobs = await listRoutines();
      if (routineLoadSequenceRef.current !== sequence) return null;
      setRoutines(sortRoutines(jobs));
      setError(null);
      setErrorRetryable(false);
      return null;
    } catch (err) {
      const message = describeRoutineError(err);
      if (routineLoadSequenceRef.current !== sequence || silent) return message;
      setError(message);
      setErrorRetryable(true);
      return message;
    } finally {
      if (routineLoadSequenceRef.current === sequence) setLoading(false);
      if (visibleSequence !== null) {
        visibleRoutineLoadsInFlightRef.current = Math.max(
          0,
          visibleRoutineLoadsInFlightRef.current - 1,
        );
        if (visibleRoutineLoadSequenceRef.current === visibleSequence) {
          setRefreshing(false);
        }
      }
    }
  }, []);

  // Run history comes from a different local store (the session store, not the
  // cron manager), so its failure must not take the routines list down with
  // it — it degrades to a quiet notice inside the section instead.
  const loadRuns = useCallback(async () => {
    const sequence = runLoadSequenceRef.current + 1;
    runLoadSequenceRef.current = sequence;
    try {
      const nextRuns = await listRoutineRunSessions({ includeActive: true });
      if (runLoadSequenceRef.current !== sequence) return;
      setRuns(nextRuns);
      setRunsUnavailable(false);
      setRunsLoaded(true);
      void creditApprovalRuns(nextRuns, reportedRunsRef.current);
    } catch {
      if (runLoadSequenceRef.current !== sequence) return;
      setRunsUnavailable(true);
      setRunsLoaded(true);
    }
  }, []);

  const refresh = useCallback(
    () => Promise.all([loadRoutines(), loadRuns()]),
    [loadRoutines, loadRuns],
  );
  const refreshSilently = useCallback(
    () => Promise.all([loadRoutines({ silent: true }), loadRuns()]),
    [loadRoutines, loadRuns],
  );

  useClovyDataChanged(
    "routines",
    () => {
      void refreshSilently();
    },
    true,
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<AgentRuntimeEvent>(AGENT_RUNTIME_EVENT, ({ payload }) => {
      if (
        payload.method !== "run.completed" &&
        payload.method !== "run.cancelled" &&
        payload.method !== "run.failed"
      ) {
        return;
      }
      // The native runtime projects terminal agent state into both the
      // routine and session stores before emitting this event. Refresh both
      // projections now so an open detail page cannot keep showing the
      // previous run's failure until the user navigates away.
      void refreshSilently();
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      })
      .catch(() => {
        // The periodic refresh remains the fallback when event registration is
        // unavailable, including outside a native Tauri event environment.
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refreshSilently]);

  useEffect(() => {
    if (forcedEmpty) return;
    const timer = window.setInterval(() => void refreshSilently(), RUN_HISTORY_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [forcedEmpty, refreshSilently]);

  const filtered = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return routines;
    return routines.filter((routine) =>
      // Match the displayed wording too, so "weekdays" finds a routine whose
      // stored schedule is "0 9 * * 1-5".
      `${routine.name} ${routine.prompt_preview} ${routine.schedule} ${humanizeSchedule(routine.schedule)} ${compactScheduleLabel(routine.schedule)}`
        .toLowerCase()
        .includes(normalized),
    );
  }, [routines, query]);

  const routinesById = useMemo(
    () => new Map(routines.map((routine) => [routine.job_id, routine])),
    [routines],
  );

  // A run is labeled with its routine's current name; once the routine is
  // deleted, the session's own derived title is the best label left.
  const runLabel = useCallback(
    (run: RoutineRunSession) => {
      const jobId = routineIdFromSession(run);
      const routine = jobId ? routinesById.get(jobId) : undefined;
      const sessionTitle = isReplaceableRoutineRunTitle(run.title) ? "" : run.title?.trim();
      return routine?.name || sessionTitle || t("routines.run.fallbackLabel");
    },
    [routinesById, t],
  );

  const filteredRuns = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return runs;
    return runs.filter((run) =>
      `${runLabel(run)} ${run.title ?? ""} ${run.preview ?? ""}`.toLowerCase().includes(normalized),
    );
  }, [runs, query, runLabel]);

  function markBusy(jobId: string, busy: boolean) {
    setBusyIds((current) => {
      const next = new Set(current);
      if (busy) next.add(jobId);
      else next.delete(jobId);
      return next;
    });
  }

  async function toggleActive(routine: RoutineJob) {
    markBusy(routine.job_id, true);
    try {
      if (routine.state === "paused") await resumeRoutine(routine.job_id);
      else await pauseRoutine(routine.job_id);
      // loadRoutines manages the error banner itself (clears on success,
      // sets on failure) — clearing here would mask a failed reload.
      const reloadError = await loadRoutines();
      setDetailError(reloadError);
      setDetailErrorRetryable(reloadError !== null);
    } catch (err) {
      const message = describeRoutineError(err);
      setError(message);
      setErrorRetryable(false);
      setDetailError(message);
      setDetailErrorRetryable(false);
    } finally {
      markBusy(routine.job_id, false);
    }
  }

  async function runNow(routine: RoutineJob) {
    markBusy(routine.job_id, true);
    try {
      await triggerRoutine(routine.job_id);
      setDetailError(null);
      setDetailErrorRetryable(false);
    } catch (err) {
      setDetailError(describeRoutineError(err));
      setDetailErrorRetryable(false);
      throw err;
    } finally {
      markBusy(routine.job_id, false);
    }
  }

  async function saveRoutine(jobId: string, updates: RoutineUpdates) {
    setSaving(true);
    try {
      await updateRoutine(jobId, updates);
      const reloadError = await loadRoutines();
      setDetailError(reloadError);
      setDetailErrorRetryable(reloadError !== null);
    } catch (err) {
      setDetailError(describeRoutineError(err));
      setDetailErrorRetryable(false);
      // Re-throw so RoutineDetail can roll back the trust/grant change it made
      // before this cron update. Swallowing the failure would leave the DB
      // trust and the job's enabled_toolsets inconsistent (a downgrade could
      // delete the grant while the job kept its old autonomous toolsets).
      throw err;
    } finally {
      setSaving(false);
    }
  }

  async function submitCreate(input: RoutineCreateInput) {
    setCreating(true);
    try {
      const eventTrigger = input.trigger.source !== "schedule" ? input.trigger : null;
      // A routine is connector-aware when anything about it touches Google:
      // a non-default trust mode, an event trigger, or a connector template's
      // scope requirements. Plain routines do not need a trust record, but
      // they still persist the native base catalog as an explicit grant.
      const connectorAware =
        input.trustMode !== "read_only" ||
        eventTrigger !== null ||
        (input.connectorScopes?.length ?? 0) > 0;
      let created: RoutineJob;
      if (connectorAware) {
        if (!policy) {
          setCreateError(t("routines.policyLoading"));
          return;
        }
        created = await createRoutine({
          prompt: input.prompt,
          // Event routines still need a schedule record underneath. A
          // far-future one-time schedule hands firing to the trigger daemon.
          schedule: eventTrigger ? eventTriggerScheduleDraft().schedule : input.schedule,
          name: input.name,
          enabledToolsets: routineToolsetsFor(policy, input.trustMode, {
            unrestricted: input.unrestricted,
          }),
        });
      } else {
        created = await createRoutine({
          prompt: input.prompt,
          schedule: input.schedule,
          name: input.name,
          unrestricted: input.unrestricted,
          enabledToolsets: input.unrestricted
            ? UNRESTRICTED_ROUTINE_TOOLSETS
            : (policy?.routine.sandboxedBaseToolsets ?? []),
        });
      }
      if (connectorAware) {
        try {
          await routineTrustSet({
            jobId: created.job_id,
            trustMode: input.trustMode,
            autonomousTools: input.trustMode === "autonomous" ? input.autonomousTools : undefined,
          });
          if (eventTrigger && input.triggerAccountId) {
            // Subscribing is required setup, not best-effort. If it fails,
            // removeRoutine below deletes the Clovy routine and all
            // connector rows so retrying cannot create a duplicate or leave a
            // dormant 2099 placeholder behind.
            await connectorTriggerSet({
              jobId: created.job_id,
              kind: eventTrigger.source,
              accountId: input.triggerAccountId,
              config: triggerConfigFromDraft(eventTrigger),
            });
          }
        } catch (setupError) {
          try {
            await removeRoutine(created.job_id);
          } catch (cleanupError) {
            throw new Error(
              t("routines.view.cleanupFailed", {
                error: describeRoutineError(setupError),
                cleanupError: describeRoutineError(cleanupError),
              }),
            );
          }
          throw setupError;
        }
        // The first run fires right away (still under the chosen trust mode, so
        // any actions wait for approval), so an install shows value in the first
        // session instead of waiting for a future email or calendar event. This
        // one-off trigger leaves the routine active, so its event trigger can
        // keep firing later. The far-future schedule prevents timer dispatch.
        // Best-effort.
        await triggerRoutine(created.job_id).catch(() => {});
      }
      await loadRoutines();
      setCreateError(null);
      setDetailError(null);
      setDetailErrorRetryable(false);
      setPage({ kind: "detail", jobId: created.job_id });
    } catch (err) {
      setCreateError(describeRoutineError(err));
    } finally {
      setCreating(false);
    }
  }

  async function confirmDelete() {
    const routine = pendingDelete;
    if (!routine) return;
    // ConfirmDialog swallows a thrown error (it only keeps itself open), so
    // route failures to the banner like toggleActive does instead.
    try {
      await removeRoutine(routine.job_id);
      setRoutines((prev) => prev.filter((entry) => entry.job_id !== routine.job_id));
      setError(null);
      setErrorRetryable(false);
      setPage((current) =>
        current.kind === "detail" && current.jobId === routine.job_id ? { kind: "list" } : current,
      );
    } catch (err) {
      const message = describeRoutineError(err);
      setError(message);
      setErrorRetryable(false);
      setDetailError(message);
      setDetailErrorRetryable(false);
    }
  }

  function openCreate(template?: RoutineTemplate) {
    setCreateError(null);
    setPage({ kind: "create", template });
  }

  function openDetail(routine: RoutineJob) {
    setDetailError(null);
    setDetailErrorRetryable(false);
    setPage({ kind: "detail", jobId: routine.job_id });
  }

  function refreshNow() {
    setRefreshSpins((spins) => spins + 1);
    void refresh();
  }

  function retryListLoad() {
    setRefreshSpins((spins) => spins + 1);
    void loadRoutines();
  }

  function retryDetailLoad() {
    setRefreshSpins((spins) => spins + 1);
    void loadRoutines().then((reloadError) => {
      setDetailError(reloadError);
      setDetailErrorRetryable(reloadError !== null);
    });
  }

  const detailRoutine = page.kind === "detail" ? (routinesById.get(page.jobId) ?? null) : null;

  // A detail page whose routine vanished (deleted from another surface,
  // emptied by a reload) falls back to the list instead of a dead end.
  useEffect(() => {
    if (page.kind === "detail" && !loading && !detailRoutine) {
      setPage({ kind: "list" });
    }
  }, [page.kind, loading, detailRoutine]);

  const dialogs = (
    <>
      <ConfirmDialog
        open={pendingDelete !== null}
        onClose={() => setPendingDelete(null)}
        onConfirm={confirmDelete}
        title={t("routines.view.deleteTitle", { name: pendingDelete?.name ?? "" })}
        description={t("routines.view.deleteDescription")}
        confirmLabel={t("common.delete")}
        destructive
      />
    </>
  );

  function openRun(run: RoutineRunSession) {
    const back =
      page.kind === "detail" ? { ...page, tab: "history" as const } : { kind: "list" as const };
    setPage({ kind: "result", runId: run.id, back });
  }

  if (page.kind === "result") {
    const run = runs.find((run) => run.id === page.runId);
    return (
      <RoutineRunResult
        key={page.runId}
        run={run}
        label={run ? runLabel(run) : t("routines.run.fallbackLabel")}
        onBack={() => {
          returnFocusRunId.current = page.runId;
          setPage(page.back);
        }}
      />
    );
  }

  if (page.kind === "create") {
    return (
      <>
        <RoutineCreate
          template={page.template}
          creating={creating}
          error={createError}
          onBack={() => setPage({ kind: "list" })}
          onCreate={(input) => void submitCreate(input)}
        />
        {dialogs}
      </>
    );
  }

  if (page.kind === "detail" && detailRoutine) {
    const routineRuns = runs.filter((run) => routineIdFromSession(run) === detailRoutine.job_id);
    return (
      <>
        <RoutineDetail
          key={detailRoutine.job_id}
          routine={detailRoutine}
          initialTab={page.tab}
          runs={routineRuns}
          busy={busyIds.has(detailRoutine.job_id)}
          saving={saving}
          error={detailError}
          onBack={() => setPage({ kind: "list" })}
          onSave={(updates) => saveRoutine(detailRoutine.job_id, updates)}
          onToggleActive={() => void toggleActive(detailRoutine)}
          onRunNow={() => runNow(detailRoutine)}
          onDelete={() => setPendingDelete(detailRoutine)}
          onOpenRun={openRun}
          onRetryLoad={detailErrorRetryable ? retryDetailLoad : undefined}
          retrying={refreshing}
        />
        {dialogs}
      </>
    );
  }

  return (
    <section className="routines-workspace" aria-label={t("routines.title")}>
      <header className="folders-header">
        <div className="folders-heading">
          <h1 ref={listHeading} tabIndex={-1}>
            {t("routines.title")}
            {routines.length > 0 ? <span className="folders-count">{routines.length}</span> : null}
          </h1>
          <p className="folders-subtitle">{t("routines.view.subtitle")}</p>
        </div>
        <button type="button" className="primary-action primary-solid" onClick={() => openCreate()}>
          <IconPlusMedium size={13} />
          {t("routines.newRoutine")}
        </button>
      </header>

      {routines.length > 0 ? (
        <div className="folders-controls">
          <label className="folders-search">
            <IconMagnifyingGlass size={14} />
            <input
              type="search"
              placeholder={t("common.search")}
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
            />
          </label>
          <button
            type="button"
            className="icon-button routines-refresh"
            aria-label={t("routines.view.refresh")}
            aria-busy={refreshing}
            disabled={refreshing}
            title={t("routines.view.refresh")}
            onClick={refreshNow}
          >
            <IconArrowRotateClockwise
              size={14}
              className="balance-refresh-icon"
              style={{ transform: `rotate(${refreshSpins * 360}deg)` }}
            />
          </button>
        </div>
      ) : null}

      {error ? (
        <RoutineErrorBanner
          message={error}
          onRetry={errorRetryable ? retryListLoad : undefined}
          retrying={refreshing}
        />
      ) : null}

      {loading ? (
        <div className="folders-empty">
          <p>{t("routines.view.loading")}</p>
        </div>
      ) : routines.length === 0 ? (
        <div className="routines-hero">
          <TemplateGrid onPick={openCreate} />
        </div>
      ) : filtered.length === 0 ? (
        <div className="folders-empty">
          <p>{t("routines.view.noMatch", { query: query.trim() })}</p>
        </div>
      ) : (
        <ul className="routines-list" aria-label={t("routines.title")}>
          {filtered.map((routine) => (
            <RoutineRow
              key={routine.job_id}
              policy={policy}
              routine={routine}
              busy={busyIds.has(routine.job_id)}
              onOpen={() => openDetail(routine)}
              onRunNow={() =>
                void runNow(routine).catch((err) => {
                  setError(describeRoutineError(err));
                  setErrorRetryable(false);
                })
              }
              onDelete={() => setPendingDelete(routine)}
            />
          ))}
        </ul>
      )}

      {/* Hidden while everything is empty (the hero owns the page) and while
       * a search matches no runs; shown otherwise, including when only
       * orphaned runs of deleted routines remain. */}
      {!loading &&
      (query.trim()
        ? filteredRuns.length > 0
        : routines.length > 0 || runs.length > 0 || runsUnavailable) ? (
        <section className="routines-runs" aria-label={t("routines.runHistory")}>
          <header className="routines-runs-header">
            <h2>
              {t("routines.runHistory")}
              {runs.length > 0 ? <span className="folders-count">{runs.length}</span> : null}
            </h2>
          </header>
          {runsUnavailable ? (
            <p className="routines-runs-empty">{t("routines.view.runsUnavailable")}</p>
          ) : runs.length === 0 ? (
            <p className="routines-runs-empty">{t("routines.view.noRuns")}</p>
          ) : (
            <div className="routines-runs-panel">
              <RoutineRunList runs={filteredRuns} label={runLabel} onOpen={openRun} />
            </div>
          )}
        </section>
      ) : null}

      {!loading && routines.length > 0 && !query.trim() ? (
        <section className="routines-starters" aria-label={t("routines.view.starters")}>
          <header className="routines-section-header">
            <h2>{t("routines.view.starters")}</h2>
          </header>
          <TemplateGrid onPick={openCreate} />
        </section>
      ) : null}

      {dialogs}
    </section>
  );
}

function TemplateGrid({ onPick }: { onPick: (template: RoutineTemplate) => void }) {
  const t = useT();
  return (
    <ul className="routines-template-grid">
      {ROUTINE_TEMPLATES.map((template) => (
        <li key={template.id} className="routines-template-card">
          <span className="routines-template-icon" aria-hidden>
            <template.icon size={15} />
          </span>
          <div className="routines-template-body">
            <span className="routines-template-name">
              {template.name}
              {template.unrestricted ? (
                // The list rows spell the badge out; cards just flash the
                // warm shield and let the tip carry the explanation.
                <HoverTip
                  tip={t("routines.view.starterUnrestrictedTip")}
                  className="routines-item-badge routines-item-badge-warm routines-badge-compact"
                  tabIndex={0}
                  aria-label={t("routines.mode.unrestricted")}
                >
                  <IconShieldCrossed size={11} aria-hidden />
                </HoverTip>
              ) : null}
            </span>
            <p className="routines-template-description">{template.description}</p>
            {template.toolSummary ? (
              <p className="routines-template-tools">
                {template.toolSummary}
                {template.trustMode ? (
                  <span className="routines-template-trust">
                    {" "}
                    {t("routines.trust.summary", {
                      trust: TRUST_MODE_META[template.trustMode].label.toLowerCase(),
                    })}
                  </span>
                ) : null}
              </p>
            ) : null}
          </div>
          <button
            type="button"
            className="icon-button routines-template-add"
            aria-label={t("routines.view.addTemplate", { name: template.name })}
            onClick={() => onPick(template)}
          >
            <IconPlusMedium size={13} aria-hidden />
          </button>
        </li>
      ))}
    </ul>
  );
}

function RoutineRow({
  routine,
  policy,
  busy,
  onOpen,
  onRunNow,
  onDelete,
}: {
  routine: RoutineJob;
  policy: ConnectorPolicyCatalog | null;
  busy: boolean;
  onOpen: () => void;
  onRunNow: () => void;
  onDelete: () => void;
}) {
  const t = useT();
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const paused = routine.state === "paused";
  const completed = routine.state === "completed";
  // Derived from the stored toolset override (no per-row round trip); only
  // the action-capable modes get a badge — ambient read access is every
  // routine's baseline and would read as noise.
  const trustMode = policy ? routineTrustModeFromToolsets(policy, routine.enabled_toolsets) : null;
  const trustBadge = trustMode === "approval" || trustMode === "autonomous" ? trustMode : null;
  const status = paused ? t("routines.paused") : completed ? t("routines.completed") : null;
  const activity =
    completed && routine.last_run_at
      ? t("routines.lastRan", { time: formatRunTime(routine.last_run_at) })
      : null;

  useEffect(() => {
    if (!menuOpen) return;
    function close(event: MouseEvent) {
      if (!menuRef.current?.contains(event.target as Node)) setMenuOpen(false);
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setMenuOpen(false);
    }
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  return (
    <li
      className="routines-item"
      data-state={routine.state}
      data-has-actions="true"
      data-menu-open={menuOpen || undefined}
    >
      <button type="button" className="routines-item-open" onClick={onOpen}>
        <span className="routines-item-icon" aria-hidden>
          <IconZap size={14} />
        </span>
        <span className="routines-item-body">
          <span className="routines-item-title">
            <span className="routines-item-name">{routine.name}</span>
            {routineUnrestricted(routine) ? (
              <HoverTip
                tip={t("routines.unrestrictedTip")}
                className="routines-item-badge routines-item-badge-warm"
                tabIndex={0}
              >
                <IconShieldCrossed size={11} aria-hidden />
                {t("routines.mode.unrestricted")}
              </HoverTip>
            ) : null}
            {trustBadge ? (
              <HoverTip
                tip={TRUST_MODE_META[trustBadge].description}
                className="routines-item-badge routines-item-badge-trust"
                tabIndex={0}
              >
                {TRUST_MODE_META[trustBadge].label}
              </HoverTip>
            ) : null}
            {routine.last_status === "error" ? (
              <span className="routines-item-badge routines-item-badge-error">
                {t("routines.view.lastRunFailed")}
              </span>
            ) : null}
          </span>
        </span>
        <span className="routines-item-meta">
          <span className="routine-meta-pill">
            <IconCalendarRepeat size={12} aria-hidden />
            {compactScheduleLabel(routine.schedule)}
          </span>
          {activity ? <span className="routine-meta-pill">{activity}</span> : null}
          {status ? (
            <span className="routine-meta-pill">
              {paused ? <IconPause size={12} aria-hidden /> : null}
              {status}
            </span>
          ) : null}
        </span>
      </button>
      <span className="routines-item-actions">
        <span className="routines-item-menu-wrap" ref={menuRef}>
          <button
            type="button"
            className="icon-button routines-item-menu-trigger"
            aria-label={t("routines.view.actionsFor", { name: routine.name })}
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            onClick={() => setMenuOpen((open) => !open)}
          >
            <IconDotGrid1x3Horizontal size={13} />
          </button>
          {menuOpen ? (
            <span className="sidebar-identity-menu routines-action-menu" role="menu">
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenuOpen(false);
                  onOpen();
                }}
              >
                <IconPencil size={14} />
                {t("common.edit")}
              </button>
              <button
                type="button"
                role="menuitem"
                disabled={busy || routine.state !== "scheduled"}
                onClick={() => {
                  setMenuOpen(false);
                  onRunNow();
                }}
              >
                <IconPlay size={14} />
                {t("routines.runNow")}
              </button>
              <hr className="context-menu-separator" />
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
            </span>
          ) : null}
        </span>
      </span>
    </li>
  );
}

/** Active routines first (soonest run on top), then paused, then completed. */
function sortRoutines(jobs: RoutineJob[]) {
  const rank = { scheduled: 0, paused: 1, completed: 2 } as const;
  return [...jobs].sort((left, right) => {
    const byState = (rank[left.state] ?? 0) - (rank[right.state] ?? 0);
    if (byState !== 0) return byState;
    return timeValue(left.next_run_at) - timeValue(right.next_run_at);
  });
}

function timeValue(iso: string | null | undefined) {
  if (!iso) return Number.MAX_SAFE_INTEGER;
  const time = new Date(iso).getTime();
  return Number.isNaN(time) ? Number.MAX_SAFE_INTEGER : time;
}

function describeRoutineError(err: unknown) {
  const message = messageFromError(err);
  if (/\bAPI returned 5\d\d\b/i.test(message)) {
    return translate("routines.view.serverError");
  }
  return message || translate("routines.view.unavailable");
}

function RoutineErrorBanner({
  message,
  onRetry,
  retrying,
}: {
  message: string;
  onRetry?: () => void;
  retrying?: boolean;
}) {
  const t = useT();
  return (
    <div className="error-banner routines-error-banner" role="alert">
      <p>{message}</p>
      {onRetry ? (
        <button
          type="button"
          className="btn btn-ghost"
          onClick={onRetry}
          disabled={retrying}
          aria-busy={retrying || undefined}
        >
          <IconArrowRotateClockwise size={14} className="balance-refresh-icon" aria-hidden />
          {t("common.tryAgain")}
        </button>
      ) : null}
    </div>
  );
}
