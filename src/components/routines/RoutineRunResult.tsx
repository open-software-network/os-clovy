import { useEffect, useRef, useState } from "react";
import { useT } from "../../i18n";
import type { AgentSessionStatus } from "../../lib/agent-runtime-contract";
import { type RoutineRunSession, sessionTimestamp } from "../../lib/agent-routine-history";
import { agentRuntimeBindings } from "../../lib/tauri";
import { MarkdownContent } from "../agent/MarkdownContent";
import { BreadcrumbBar } from "../ui/BreadcrumbBar";
import { formatRunTime } from "./RoutineRunList";

const statusKeys = {
  idle: "routines.run.idle",
  running: "routines.run.running",
  waiting_for_user: "routines.run.waiting",
  completed: "routines.run.finished",
  interrupted: "routines.run.interrupted",
  failed: "routines.run.failed",
} as const satisfies Record<AgentSessionStatus, string>;

export function RoutineRunResult({
  run,
  label,
  onBack,
}: {
  run: RoutineRunSession | undefined;
  label: string;
  onBack: () => void;
}) {
  const t = useT();
  const heading = useRef<HTMLHeadingElement>(null);
  const runId = run?.id;
  const updatedAt = run?.updatedAt;
  const [result, setResult] = useState<
    | { output: { id: string; text: string }[]; errors: { id: string; text: string }[] }
    | "unavailable"
  >();

  useEffect(() => {
    heading.current?.focus();
  }, []);

  useEffect(() => {
    if (!runId) return;
    let current = true;
    void agentRuntimeBindings
      .listItems(runId)
      .then((items) => {
        if (!current) return;
        const ordered = [...items].sort((left, right) => left.sequence - right.sequence);
        setResult({
          output: ordered.flatMap((item) =>
            item.kind === "message" && item.role === "assistant" && item.text.trim()
              ? [{ id: item.id, text: item.text }]
              : [],
          ),
          errors: ordered.flatMap((item) =>
            item.kind === "error" ? [{ id: item.id, text: item.message }] : [],
          ),
        });
      })
      .catch(() => {
        if (current) setResult("unavailable");
      });
    return () => {
      current = false;
    };
  }, [runId, updatedAt]);

  const runResult = result === "unavailable" ? undefined : result;
  const error = run?.error?.trim();
  const errors =
    error && !runResult?.errors.some((item) => item.text === error)
      ? [...(runResult?.errors ?? []), { id: "run-error", text: error }]
      : (runResult?.errors ?? []);

  return (
    <section className="routine-detail" aria-label={t("routines.run.result")}>
      <BreadcrumbBar
        backLabel={t("routines.backToRoutines")}
        onBack={onBack}
        items={[{ label: t("routines.title"), onClick: onBack }, { label }]}
      />
      <div className="routine-detail-content">
        <h1 ref={heading} tabIndex={-1} className="routine-detail-name">
          {t("routines.run.result")}
        </h1>
        {run ? (
          <div className="routine-detail-meta">
            <span className="routine-meta-pill" role="status">
              {t(statusKeys[run.status])}
            </span>
            <span className="routine-meta-pill">{formatRunTime(sessionTimestamp(run))}</span>
          </div>
        ) : null}
        <div className="routine-detail-body">
          {errors.map((item) => (
            <p key={item.id} className="error-banner" role="alert">
              {item.text}
            </p>
          ))}
          {!run || result === "unavailable" ? (
            <p role="status">{t("routines.run.unavailableResult")}</p>
          ) : !runResult ? (
            <p role="status">{t("routines.run.loadingResult")}</p>
          ) : runResult.output.length > 0 ? (
            runResult.output.map((item) => <MarkdownContent key={item.id} markdown={item.text} />)
          ) : errors.length === 0 ? (
            <p>{t("routines.run.noResult")}</p>
          ) : null}
        </div>
      </div>
    </section>
  );
}
