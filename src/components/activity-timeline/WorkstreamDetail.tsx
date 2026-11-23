import { IconChevronLeftSmall } from "central-icons/IconChevronLeftSmall";
import { Fragment } from "react";
import { useLocale, useT } from "../../i18n";
import { formatDuration } from "./timeline-utils";
import type { TimelineSpan } from "./workstream-timeline-layout";
import { spanLabels } from "./WorkstreamTimeline";

type WorkstreamDetailProps = {
  card: TimelineSpan;
  onBack: () => void;
};

/**
 * A selected workstream beside the grid: its active time, each sitting with
 * the breaks between them (or whole hours for records known only by hour),
 * and what was done, hour by hour.
 */
export function WorkstreamDetail({ card, onBack }: WorkstreamDetailProps) {
  const t = useT();
  const locale = useLocale();
  const { workstream } = card;
  const notes = workstream.hours.filter((hour) => hour.note);

  return (
    <section className="workstream-detail" aria-label={t("activity.workstream.details")}>
      <button type="button" className="workstream-detail-back" onClick={onBack}>
        <IconChevronLeftSmall size={14} aria-hidden="true" />
        {t("activity.workstream.back")}
      </button>
      <div className="workstream-detail-heading">
        <span className="workstream-detail-kicker">{t("activity.workstream.kicker")}</span>
        <h3 className="workstream-detail-title">{workstream.title}</h3>
        <span className="workstream-detail-meta">
          <span className="workstream-detail-active">
            {t("activity.grid.active", {
              duration: formatDuration(card.activeMs, locale),
            })}
          </span>
          <span>{t("activity.detail.timeRange", spanLabels(card, locale))}</span>
        </span>
      </div>
      {workstream.summary ? (
        <p className="workstream-detail-summary">{workstream.summary}</p>
      ) : null}

      <div className="workstream-detail-section">
        <h4 className="workstream-detail-section-title">{t("activity.workstream.when")}</h4>
        <p className="workstream-detail-hint">{t("activity.workstream.whenHint")}</p>
        <ol className="workstream-detail-periods">
          {card.segments.map((segment, index) => {
            // A break is time with none of this workstream's work, which is
            // known only when all of it was located in sessions.
            const reachedMs = Math.max(
              ...card.segments.slice(0, index).map((earlier) => earlier.endMs),
            );
            const gapMs = !card.approximate && index > 0 ? segment.startMs - reachedMs : 0;
            return (
              <Fragment key={`${segment.kind}-${segment.startMs}`}>
                {gapMs > 0 ? (
                  <li className="workstream-detail-break">
                    {t("activity.workstream.break", { duration: formatDuration(gapMs, locale) })}
                  </li>
                ) : null}
                <li
                  className="workstream-detail-period"
                  data-approximate={segment.approximate || undefined}
                  data-testid="workstream-period"
                >
                  <span className="workstream-detail-period-range">
                    {t("activity.detail.timeRange", spanLabels(segment, locale))}
                  </span>
                  {segment.kind === "session" ? null : (
                    <>
                      <span className="workstream-detail-period-length">
                        {t(
                          segment.kind === "hour"
                            ? "activity.workstream.hourActive"
                            : "activity.workstream.hourUnlocated",
                          {
                            duration: formatDuration(
                              (segment.seconds != null
                                ? segment.seconds
                                : (segment.minutes ?? 0) * 60) * 1000,
                              locale,
                            ),
                          },
                        )}
                      </span>
                      <span className="ws-timeline-badge" title={t("activity.grid.byHourHint")}>
                        {t("activity.grid.byHour")}
                      </span>
                    </>
                  )}
                </li>
              </Fragment>
            );
          })}
        </ol>
      </div>

      {notes.length > 0 ? (
        <div className="workstream-detail-section">
          <h4 className="workstream-detail-section-title">{t("activity.workstream.done")}</h4>
          <ul className="workstream-detail-notes">
            {notes.map((hour) => (
              <li key={hour.hour}>{hour.note}</li>
            ))}
          </ul>
        </div>
      ) : null}
    </section>
  );
}
