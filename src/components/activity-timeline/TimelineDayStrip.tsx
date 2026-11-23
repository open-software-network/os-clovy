import { useMemo } from "react";
import { useLocale, useT } from "../../i18n";
import type { TimelineGapDto, TimelineSessionDto } from "../../lib/activity-timeline";
import { EXTRA_TIMELINE_LANES } from "./lanes";
import {
  CATEGORY_COLOR_VARS,
  CATEGORY_LABEL_KEYS,
  formatDuration,
  formatTime,
  gapLabelKey,
  hourTicks,
} from "./timeline-utils";

type TimelineDayStripProps = {
  sessions: TimelineSessionDto[];
  gaps: TimelineGapDto[];
  dayStartMs: number;
  dayEndMs: number;
  from: string;
  to: string;
  selectedSessionId: number | null;
  onSelectSession: (id: number) => void;
  nowMs: number;
  isToday: boolean;
};

export function TimelineDayStrip({
  sessions,
  gaps,
  dayStartMs,
  dayEndMs,
  from,
  to,
  selectedSessionId,
  onSelectSession,
  nowMs,
  isToday,
}: TimelineDayStripProps) {
  const t = useT();
  const locale = useLocale();
  const dayDurationMs = Math.max(1, dayEndMs - dayStartMs);

  const xForTime = useMemo(
    () => (timeMs: number) => Math.max(0, Math.min(1, (timeMs - dayStartMs) / dayDurationMs)),
    [dayStartMs, dayDurationMs],
  );

  const ticks = useMemo(() => hourTicks(dayStartMs, dayEndMs), [dayStartMs, dayEndMs]);

  const nowFraction = isToday && nowMs >= dayStartMs && nowMs <= dayEndMs ? xForTime(nowMs) : null;

  return (
    <section className="timeline-day-strip" aria-label={t("activity.timeline.title")}>
      <div className="timeline-strip-header">
        <div className="timeline-lane-name">{t("activity.lanes.sessions")}</div>
      </div>

      <div className="timeline-strip-container">
        {/* Hour markers along the 24-hour scale */}
        <div className="timeline-time-ruler" aria-hidden="true">
          {ticks.map((tick) => (
            <div
              key={tick.ms}
              className="timeline-ruler-tick"
              data-testid="timeline-ruler-tick"
              style={{ left: `${xForTime(tick.ms) * 100}%` }}
            >
              <span className="timeline-ruler-label">{tick.label}</span>
            </div>
          ))}
        </div>

        {/* Primary Sessions & Gaps Track */}
        <div className="timeline-track timeline-sessions-track">
          {/* Gaps */}
          {gaps.map((gap) => {
            const startMs = new Date(gap.startedAt).getTime();
            const endMs = new Date(gap.endedAt).getTime();
            const left = xForTime(startMs);
            const right = xForTime(endMs);
            const width = Math.max(0.001, right - left);
            const label = t(gapLabelKey(gap.kind, gap.pauseReason));
            const durationStr = formatDuration(gap.durationMs, locale);
            const timeStr = `${formatTime(gap.startedAt, locale)} - ${formatTime(gap.endedAt, locale)}`;

            return (
              <div
                key={`gap-${gap.id}-${gap.startedAt}`}
                className="timeline-gap-block"
                data-gap-kind={gap.kind}
                data-pause-reason={gap.pauseReason ?? undefined}
                style={{
                  left: `${left * 100}%`,
                  width: `${width * 100}%`,
                }}
                title={`${label} (${timeStr}, ${durationStr})`}
              />
            );
          })}

          {/* Sessions */}
          {sessions.map((session) => {
            const startMs = new Date(session.startedAt).getTime();
            const endMs = new Date(session.endedAt).getTime();
            const left = xForTime(startMs);
            const right = xForTime(endMs);
            // Minimum 3px or 0.25% so small sessions remain easily visible and clickable
            const width = Math.max(0.0025, right - left);
            const categoryColor = CATEGORY_COLOR_VARS[session.category] ?? "var(--brand)";
            const categoryLabel = t(CATEGORY_LABEL_KEYS[session.category]);
            const isSelected = selectedSessionId === session.id;
            const durationStr = formatDuration(session.durationMs, locale);
            const timeStr = `${formatTime(session.startedAt, locale)} - ${formatTime(session.endedAt, locale)}`;
            const tooltip = `${session.appName}: ${session.windowTitle ?? categoryLabel} (${timeStr}, ${durationStr})`;

            return (
              <button
                key={`session-${session.id}`}
                type="button"
                className="timeline-session-block"
                data-active-session={session.active || undefined}
                data-selected={isSelected || undefined}
                style={{
                  left: `${left * 100}%`,
                  width: `${width * 100}%`,
                  backgroundColor: categoryColor,
                }}
                title={tooltip}
                aria-label={tooltip}
                aria-pressed={isSelected}
                onClick={() => onSelectSession(session.id)}
              >
                {session.active ? (
                  <span className="timeline-live-indicator" aria-hidden="true" />
                ) : null}
              </button>
            );
          })}

          {/* Now cursor line */}
          {nowFraction !== null ? (
            <div
              className="timeline-now-cursor"
              style={{ left: `${nowFraction * 100}%` }}
              title={t("activity.timeline.live")}
              aria-hidden="true"
            />
          ) : null}
        </div>

        {/* Extra Lanes from Extension Point */}
        {EXTRA_TIMELINE_LANES.map((lane) => (
          <div key={lane.id} className="timeline-extra-lane">
            <div className="timeline-strip-header">
              <div className="timeline-lane-name">{t(lane.label)}</div>
            </div>
            <div className="timeline-track timeline-extra-track">
              {lane.render({ from, to, dayStartMs, dayEndMs, xForTime })}
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}
