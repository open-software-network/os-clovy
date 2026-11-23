import { IconZap } from "central-icons/IconZap";
import {
  isRunningRoutineSession,
  sessionTimestamp,
  type RoutineRunSession,
} from "../../lib/agent-routine-history";
import { clockTimeOptions, formatDate, t as translate, useT } from "../../i18n";
import { isSameLocalDay } from "../../lib/date-format";

/** Past runs of one or all routines. Each row opens the run result. */
export function RoutineRunList({
  runs,
  label,
  onOpen,
}: {
  runs: RoutineRunSession[];
  label: (run: RoutineRunSession) => string;
  onOpen: (run: RoutineRunSession) => void;
}) {
  return (
    <ul className="routines-list routines-runs-list">
      {runs.map((run) => (
        <RunRow key={run.id} run={run} label={label(run)} onOpen={() => onOpen(run)} />
      ))}
    </ul>
  );
}

function RunRow({
  run,
  label,
  onOpen,
}: {
  run: RoutineRunSession;
  label: string;
  onOpen: () => void;
}) {
  const t = useT();
  const running = isRunningRoutineSession(run);
  const preview = run.preview?.trim() || (running ? t("routines.run.runningNow") : "");
  return (
    <li className="routines-run">
      <button
        id={`routine-run-${run.id}`}
        type="button"
        className="routines-run-button"
        onClick={onOpen}
      >
        <span className="routines-item-icon" aria-hidden>
          <IconZap size={14} />
        </span>
        <span className="routines-run-body">
          <span className="routines-run-title">
            <span className="routines-run-name">{label}</span>
            {running ? (
              <span className="routines-run-status">{t("routines.run.running")}</span>
            ) : null}
          </span>
          {preview ? <span className="routines-run-preview">{preview}</span> : null}
        </span>
        <span className="routines-run-time">{formatRunTime(sessionTimestamp(run))}</span>
      </button>
    </li>
  );
}

export function formatRunTime(iso: string) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const now = new Date();
  const time = formatDate(date, {
    ...clockTimeOptions(),
  });
  if (isSameLocalDay(date, now)) return translate("routines.run.today", { time });
  const tomorrow = new Date(now);
  tomorrow.setDate(now.getDate() + 1);
  if (isSameLocalDay(date, tomorrow)) return translate("routines.run.tomorrow", { time });
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (isSameLocalDay(date, yesterday)) return translate("routines.run.yesterday", { time });
  return formatDate(date, {
    month: "short",
    day: "numeric",
    ...clockTimeOptions(),
  });
}
