import { useEffect, useRef, useState } from "react";
import { IconArrowRotateClockwise } from "central-icons/IconArrowRotateClockwise";
import { IconCircleInfo } from "central-icons/IconCircleInfo";
import { IconSettingsGear4 } from "central-icons/IconSettingsGear4";
import { IconSparkle } from "central-icons/IconSparkle";
import { type MessageKey, useLocale, useT } from "../../i18n";
import {
  type DayPanelsDto,
  formatStandup,
  generateDaySummary,
  type HourReportDto,
  type WorkstreamDto,
} from "../../lib/day-intelligence";
import { useScrollFade } from "../../lib/use-scroll-fade";
import { CopyStateIcon } from "../ui/CopyStateIcon";
import {
  CATEGORY_COLOR_VARS,
  CATEGORY_LABEL_KEYS,
  formatDuration,
  formatTime,
} from "./timeline-utils";
import { useDayIntelligence } from "./use-day-intelligence";

export type DaySummaryPanelProps = {
  /** Local day, "YYYY-MM-DD". */
  day: string;
  onNavigateToSettings?: (tab: "activity" | "models") => void;
};

function errorMessage(error: unknown): string {
  if (
    error &&
    typeof error === "object" &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return error.message;
  }
  return String(error);
}

/** Refusals that keep a summary from being written read in the interface language. */
const GENERATE_ERROR_KEYS: Record<string, MessageKey> = {
  day_summary_incomplete: "dayIntelligence.errors.incomplete",
  day_summary_no_activity: "dayIntelligence.errors.noActivity",
};

function errorCode(error: unknown): string | undefined {
  if (error && typeof error === "object" && "code" in error && typeof error.code === "string") {
    return error.code;
  }
  return undefined;
}

/** "cli:claude" → "claude", "endpoint:local" → "local". */
function providerName(provider: string): string {
  return provider.replace(/^(cli|endpoint):/, "");
}

export function DaySummaryPanel({ day, onNavigateToSettings }: DaySummaryPanelProps) {
  const t = useT();
  const locale = useLocale();
  const { data, error: loadError, reload, replace } = useDayIntelligence(day);
  const [generating, setGenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const scrollerRef = useRef<HTMLDivElement>(null);
  const fade = useScrollFade(scrollerRef);
  const dayRef = useRef(day);
  dayRef.current = day;

  // biome-ignore lint/correctness/useExhaustiveDependencies: a generation's notices belong to its day
  useEffect(() => {
    setError(null);
    setCopied(false);
  }, [day]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: re-measure when the content changes
  useEffect(() => {
    fade.update();
  }, [data, fade.update]);

  async function handleGenerate() {
    setGenerating(true);
    setError(null);
    try {
      replace(await generateDaySummary(day));
    } catch (generateError) {
      if (dayRef.current === day) {
        const key = GENERATE_ERROR_KEYS[errorCode(generateError) ?? ""];
        setError(
          key ? t(key) : t("dayIntelligence.error", { message: errorMessage(generateError) }),
        );
      }
    } finally {
      setGenerating(false);
    }
  }

  async function handleCopyStandup() {
    if (!data?.summary) return;
    const text = formatStandup(data.summary.standup, {
      done: t("dayIntelligence.standup.done"),
      inProgress: t("dayIntelligence.standup.inProgress"),
      blockers: t("dayIntelligence.standup.blockers"),
      none: t("dayIntelligence.standup.none"),
    });
    await navigator.clipboard.writeText(text);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 2000);
  }

  if (!data) {
    return (
      <div className="day-summary-panel day-summary-loading">
        {loadError ? (
          <>
            <p className="day-summary-error" role="alert">
              {t("dayIntelligence.loadError", { message: loadError })}
            </p>
            <button type="button" className="btn btn-secondary day-summary-button" onClick={reload}>
              <IconArrowRotateClockwise size={14} aria-hidden="true" />
              {t("common.retry")}
            </button>
          </>
        ) : (
          <span className="dot-spinner" aria-hidden="true" />
        )}
      </div>
    );
  }

  const busy = generating || data.running;
  const summary = data.summary;
  const providerReady = data.provider === "ready";

  return (
    <div className="day-summary-panel scroll-fade-mask" ref={scrollerRef} {...fade.props}>
      <section className="day-summary-section" aria-labelledby="day-summary-heading">
        <header className="day-summary-header">
          <div className="day-summary-heading-group">
            <h3 id="day-summary-heading" className="day-summary-heading">
              {t("dayIntelligence.summary.title")}
            </h3>
            {summary ? (
              <p className="day-summary-meta">
                {t("dayIntelligence.generatedAt", {
                  time: formatTime(summary.generatedAt, locale),
                  provider: providerName(summary.provider),
                })}
              </p>
            ) : null}
          </div>
          <div className="day-summary-actions">
            {summary ? (
              <button
                type="button"
                className="btn btn-secondary day-summary-button"
                onClick={() => void handleCopyStandup()}
                aria-label={copied ? t("dayIntelligence.copied") : t("dayIntelligence.copyStandup")}
              >
                <CopyStateIcon copied={copied} />
                {copied ? t("dayIntelligence.copied") : t("dayIntelligence.copyStandup")}
              </button>
            ) : null}
            {providerReady ? (
              <button
                type="button"
                className={`btn day-summary-button ${summary ? "btn-secondary" : "btn-primary"}`}
                onClick={() => void handleGenerate()}
                disabled={busy}
              >
                {busy ? (
                  <span className="dot-spinner" aria-hidden="true" />
                ) : summary ? (
                  <IconArrowRotateClockwise size={14} aria-hidden="true" />
                ) : (
                  <IconSparkle size={14} aria-hidden="true" />
                )}
                {busy
                  ? t("dayIntelligence.generating")
                  : summary
                    ? t("dayIntelligence.regenerate")
                    : t("dayIntelligence.generate")}
              </button>
            ) : null}
          </div>
        </header>

        {data.provider !== "ready" ? (
          <div className="day-summary-notice" role="status">
            <IconCircleInfo size={16} aria-hidden="true" />
            <div className="day-summary-notice-text">
              <p className="day-summary-notice-title">
                {data.provider === "missing"
                  ? t("dayIntelligence.provider.missingTitle")
                  : t("dayIntelligence.provider.insufficientTitle")}
              </p>
              <p className="day-summary-notice-description">
                {data.provider === "missing"
                  ? t("dayIntelligence.provider.missingDescription")
                  : t("dayIntelligence.provider.insufficientDescription")}
              </p>
            </div>
            {onNavigateToSettings ? (
              <button
                type="button"
                className="btn btn-secondary day-summary-button"
                onClick={() => onNavigateToSettings("models")}
              >
                <IconSettingsGear4 size={14} aria-hidden="true" />
                {t("dayIntelligence.provider.choose")}
              </button>
            ) : null}
          </div>
        ) : null}

        {error ? (
          <p className="day-summary-error" role="alert">
            {error}
          </p>
        ) : null}

        {providerReady && data.embedder === "downloading" ? (
          <p className="day-summary-meta">{t("dayIntelligence.embedderDownloading")}</p>
        ) : null}

        {summary ? (
          <div className="day-summary-body">
            <p className="day-summary-headline">{summary.headline}</p>
            <p className="day-summary-narrative">{summary.narrative}</p>
            {summary.insights.length > 0 ? (
              <div className="day-summary-insights-block">
                <h4 className="day-summary-subheading">{t("dayIntelligence.insights.title")}</h4>
                <div className="day-summary-insights">
                  {summary.insights.map((insight) => (
                    <div key={`${insight.title}-${insight.text}`} className="day-summary-insight">
                      {insight.title ? (
                        <p className="day-summary-insight-title">{insight.title}</p>
                      ) : null}
                      <p className="day-summary-insight-text">{insight.text}</p>
                    </div>
                  ))}
                </div>
              </div>
            ) : null}
            <div className="day-summary-standup">
              <h4 className="day-summary-subheading">{t("dayIntelligence.standup.title")}</h4>
              <div className="day-summary-standup-grid">
                {(
                  [
                    ["dayIntelligence.standup.done", summary.standup.done],
                    ["dayIntelligence.standup.inProgress", summary.standup.inProgress],
                    ["dayIntelligence.standup.blockers", summary.standup.blockers],
                  ] as const
                ).map(([key, lines]) => (
                  <div key={key} className="day-summary-standup-column">
                    <p className="day-summary-standup-title">{t(key)}</p>
                    <ul className="day-summary-standup-list">
                      {(lines.length > 0 ? lines : [t("dayIntelligence.standup.none")]).map(
                        (line) => (
                          <li key={line}>{line}</li>
                        ),
                      )}
                    </ul>
                  </div>
                ))}
              </div>
            </div>
          </div>
        ) : providerReady ? (
          <p className="day-summary-empty">
            {t("dayIntelligence.empty", { time: data.summaryTime })}
          </p>
        ) : null}
      </section>

      <DayPanels panels={data.panels} />
      <Workstreams workstreams={data.workstreams} />
      <HourReports reports={data.hourReports} />
    </div>
  );
}

function DayPanels({ panels }: { panels: DayPanelsDto }) {
  const t = useT();
  const locale = useLocale();
  const maxHourMs = Math.max(1, ...panels.hours.map((hour) => hour.focusedMs));
  const maxWorkstream = Math.max(
    1,
    ...panels.workstreams.map((workstream) => workstream.seconds ?? workstream.minutes * 60),
  );
  const focusTotal = Math.max(1, panels.focusedMs);

  return (
    <section className="day-summary-section" aria-labelledby="day-panels-heading">
      <h4 id="day-panels-heading" className="day-summary-subheading">
        {t("dayIntelligence.panels.title")}
      </h4>
      <div className="day-panels-metrics">
        <div className="timeline-metric-card" data-panel="focused">
          <span className="timeline-metric-label">{t("activity.stats.focused")}</span>
          <span className="timeline-metric-value">{formatDuration(panels.focusedMs, locale)}</span>
        </div>
        <div className="timeline-metric-card" data-panel="idle">
          <span className="timeline-metric-label">{t("activity.stats.idle")}</span>
          <span className="timeline-metric-value">{formatDuration(panels.idleMs, locale)}</span>
        </div>
        <div className="timeline-metric-card" data-panel="away">
          <span className="timeline-metric-label">{t("activity.stats.away")}</span>
          <span className="timeline-metric-value">{formatDuration(panels.awayMs, locale)}</span>
        </div>
        <div className="timeline-metric-card" data-panel="meetings">
          <span className="timeline-metric-label">{t("dayIntelligence.panels.meetings")}</span>
          <span className="timeline-metric-value">{formatDuration(panels.meetingMs, locale)}</span>
          <span className="timeline-metric-label">
            {t("dayIntelligence.panels.meetingCount", { count: panels.meetingCount })}
          </span>
        </div>
        <div className="timeline-metric-card" data-panel="codingAgents">
          <span className="timeline-metric-label">{t("dayIntelligence.panels.codingAgents")}</span>
          <span className="timeline-metric-value">
            {formatDuration(panels.codingAgentActiveSeconds * 1000, locale)}
          </span>
          <span className="timeline-metric-label">
            {t("dayIntelligence.panels.codingAgentBlocks", { count: panels.codingAgentBlocks })}
          </span>
        </div>
      </div>

      {panels.categories.length > 0 ? (
        <div className="timeline-category-stacked-bar" aria-hidden="true">
          {panels.categories.map((category) => (
            <div
              key={category.category}
              className="timeline-stacked-segment"
              title={t(CATEGORY_LABEL_KEYS[category.category])}
              style={{
                width: `${(category.durationMs / focusTotal) * 100}%`,
                backgroundColor: CATEGORY_COLOR_VARS[category.category],
              }}
            />
          ))}
        </div>
      ) : null}

      <div className="day-panels-chart">
        <p className="timeline-stats-heading">{t("dayIntelligence.panels.byHour")}</p>
        <div className="day-panels-hours">
          {panels.hours.map((hour) => (
            <div
              key={hour.hour}
              className="day-panels-hour"
              title={`${hour.hour.slice(11, 13)}:00 ${formatDuration(hour.focusedMs, locale)}`}
            >
              <div
                className="day-panels-hour-fill"
                style={{ height: `${(hour.focusedMs / maxHourMs) * 100}%` }}
              />
            </div>
          ))}
        </div>
      </div>

      {panels.workstreams.length > 0 ? (
        <div className="day-panels-chart">
          <p className="timeline-stats-heading">{t("dayIntelligence.panels.byWorkstream")}</p>
          {panels.workstreams.map((workstream) => (
            <div key={workstream.id} className="day-panels-bar-row">
              <span className="day-panels-bar-label">{workstream.title}</span>
              <div className="timeline-metric-bar-track day-panels-bar-track">
                <div
                  className="timeline-metric-bar-fill"
                  style={{
                    width: `${((workstream.seconds ?? workstream.minutes * 60) / maxWorkstream) * 100}%`,
                    backgroundColor: "var(--brand)",
                  }}
                />
              </div>
              <span className="day-panels-bar-value">
                {formatDuration((workstream.seconds ?? workstream.minutes * 60) * 1000, locale)}
              </span>
            </div>
          ))}
        </div>
      ) : null}
    </section>
  );
}

function Workstreams({ workstreams }: { workstreams: WorkstreamDto[] }) {
  const t = useT();
  const locale = useLocale();
  return (
    <section className="day-summary-section" aria-labelledby="day-workstreams-heading">
      <h4 id="day-workstreams-heading" className="day-summary-subheading">
        {t("dayIntelligence.workstreams.title")}
      </h4>
      {workstreams.length === 0 ? (
        <p className="day-summary-empty">{t("dayIntelligence.workstreams.empty")}</p>
      ) : (
        <ul className="day-workstream-list">
          {workstreams.map((workstream) => (
            <li key={workstream.id} className="day-workstream">
              <div className="day-workstream-header">
                <span className="day-workstream-title">{workstream.title}</span>
                <span className="day-workstream-minutes">
                  {formatDuration((workstream.seconds ?? workstream.minutes * 60) * 1000, locale)}
                </span>
              </div>
              <p className="day-workstream-summary">{workstream.summary}</p>
              <div className="day-workstream-hours">
                {workstream.hours.map((hour) => (
                  <span key={hour.hour} className="day-workstream-hour">
                    {`${hour.hour.slice(11, 13)}:00 · ${formatDuration((hour.seconds ?? hour.minutes * 60) * 1000, locale)}`}
                  </span>
                ))}
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function HourReports({ reports }: { reports: HourReportDto[] }) {
  const t = useT();
  const locale = useLocale();
  return (
    <section className="day-summary-section" aria-labelledby="day-hours-heading">
      <h4 id="day-hours-heading" className="day-summary-subheading">
        {t("dayIntelligence.hours.title")}
      </h4>
      {reports.length === 0 ? (
        <p className="day-summary-empty">{t("dayIntelligence.hours.empty")}</p>
      ) : (
        <ol className="day-hour-list">
          {reports.map((report) => (
            <li key={report.hour} className="day-hour-report" data-hour={report.hour}>
              <div className="day-hour-header">
                <span className="day-hour-label">{`${report.hour.slice(11, 13)}:00`}</span>
                <span className="day-hour-minutes">
                  {formatDuration(report.activeMinutes * 60_000, locale)}
                </span>
              </div>
              <p className="day-hour-summary">{report.summary}</p>
              <ul className="day-hour-activities">
                {report.activities.map((activity) => (
                  <li key={activity.description} className="day-hour-activity">
                    <span className="day-hour-activity-minutes">
                      {formatDuration(
                        (activity.seconds != null ? activity.seconds : activity.minutes * 60) *
                          1000,
                        locale,
                      )}
                    </span>
                    <span>{activity.description}</span>
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
