import { useEffect, useMemo, useRef } from "react";
import { IconHourglass } from "central-icons/IconHourglass";
import { IconPause } from "central-icons/IconPause";
import { IconSleep } from "central-icons/IconSleep";
import { useLocale, useT } from "../../i18n";
import type { TimelineGapDto, TimelineSessionDto } from "../../lib/activity-timeline";
import { useScrollFade } from "../../lib/use-scroll-fade";
import {
  CATEGORY_COLOR_VARS,
  CATEGORY_LABEL_KEYS,
  formatDuration,
  formatTime,
  gapLabelKey,
} from "./timeline-utils";

type TimelineSessionListProps = {
  sessions: TimelineSessionDto[];
  gaps: TimelineGapDto[];
  selectedSessionId: number | null;
  onSelectSession: (id: number) => void;
};

type TimelineItem =
  | { kind: "session"; data: TimelineSessionDto }
  | { kind: "gap"; data: TimelineGapDto };

export function TimelineSessionList({
  sessions,
  gaps,
  selectedSessionId,
  onSelectSession,
}: TimelineSessionListProps) {
  const t = useT();
  const locale = useLocale();
  const scrollRef = useRef<HTMLDivElement>(null);
  const scrollFade = useScrollFade(scrollRef);

  // Chronologically interleaved list of sessions and gaps (oldest first)
  const items = useMemo(() => {
    const list: TimelineItem[] = [
      ...sessions.map((data) => ({ kind: "session" as const, data })),
      ...gaps.map((data) => ({ kind: "gap" as const, data })),
    ];
    list.sort(
      (a, b) => new Date(a.data.startedAt).getTime() - new Date(b.data.startedAt).getTime(),
    );
    return list;
  }, [sessions, gaps]);

  // Scroll the selected session into view when selected
  useEffect(() => {
    if (selectedSessionId === null) return;
    const el = document.getElementById(`timeline-session-row-${selectedSessionId}`);
    el?.scrollIntoView?.({ behavior: "smooth", block: "nearest" });
  }, [selectedSessionId]);

  return (
    <div ref={scrollRef} className="timeline-session-list scroll-fade-mask" {...scrollFade.props}>
      {items.length === 0 ? (
        <div className="timeline-empty-message">
          <p>{t("activity.timeline.emptyDay")}</p>
        </div>
      ) : (
        items.map((item) => {
          if (item.kind === "gap") {
            const gap = item.data;
            const durationStr = formatDuration(gap.durationMs, locale);
            const timeStr = `${formatTime(gap.startedAt, locale)} - ${formatTime(gap.endedAt, locale)}`;
            const label = t(gapLabelKey(gap.kind, gap.pauseReason));

            return (
              <div
                key={`gap-row-${gap.id}-${gap.startedAt}`}
                className="timeline-row timeline-gap-row"
                data-gap-kind={gap.kind}
              >
                <div className="timeline-row-icon" aria-hidden="true">
                  {gap.kind === "sleep" ? (
                    <IconSleep size={14} />
                  ) : gap.kind === "paused" ? (
                    <IconPause size={14} />
                  ) : (
                    <IconHourglass size={14} />
                  )}
                </div>
                <div className="timeline-row-body">
                  <div className="timeline-row-title-line">
                    <span className="timeline-gap-label">{label}</span>
                    <span className="timeline-row-duration">{durationStr}</span>
                  </div>
                  <div className="timeline-row-subline">
                    <span className="timeline-row-time">{timeStr}</span>
                  </div>
                </div>
              </div>
            );
          }

          const session = item.data;
          const isSelected = selectedSessionId === session.id;
          const categoryColor = CATEGORY_COLOR_VARS[session.category] ?? "var(--brand)";
          const categoryLabel = t(CATEGORY_LABEL_KEYS[session.category]);
          const durationStr = formatDuration(session.durationMs, locale);
          const timeStr = `${formatTime(session.startedAt, locale)} - ${formatTime(session.endedAt, locale)}`;

          return (
            <button
              key={`session-row-${session.id}`}
              id={`timeline-session-row-${session.id}`}
              type="button"
              className="timeline-row timeline-session-row"
              data-selected={isSelected || undefined}
              data-active-session={session.active || undefined}
              onClick={() => onSelectSession(session.id)}
            >
              <div
                className="timeline-row-category-indicator"
                style={{ backgroundColor: categoryColor }}
                aria-hidden="true"
              />
              <div className="timeline-row-body">
                <div className="timeline-row-title-line">
                  <span className="timeline-row-app">{session.appName}</span>
                  {session.context ? (
                    <span className="timeline-row-context">{session.context}</span>
                  ) : null}
                  <span
                    className="timeline-category-chip"
                    style={{
                      color: categoryColor,
                      backgroundColor: `color-mix(in oklch, ${categoryColor} 12%, var(--card))`,
                      borderColor: `color-mix(in oklch, ${categoryColor} 28%, transparent)`,
                    }}
                  >
                    {categoryLabel}
                  </span>
                  {session.active ? (
                    <span className="timeline-live-badge">{t("activity.timeline.live")}</span>
                  ) : null}
                  <span className="timeline-row-duration">{durationStr}</span>
                </div>
                <div className="timeline-row-subline">
                  <span className="timeline-row-window-title">
                    {session.windowTitle || categoryLabel}
                  </span>
                  <span className="timeline-row-time">{timeStr}</span>
                </div>
              </div>
            </button>
          );
        })
      )}
    </div>
  );
}
