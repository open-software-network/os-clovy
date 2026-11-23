import { useEffect, useRef, useState } from "react";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconGlobe } from "central-icons/IconGlobe";
import { IconWindowSparkle } from "central-icons/IconWindowSparkle";
import { useLocale, useT } from "../../i18n";
import {
  activityTimelineSession,
  type TimelineSessionDetailDto,
} from "../../lib/activity-timeline";
import { useScrollFade } from "../../lib/use-scroll-fade";
import {
  CATEGORY_COLOR_VARS,
  CATEGORY_LABEL_KEYS,
  formatDuration,
  formatTime,
} from "./timeline-utils";

type TimelineSessionDetailProps = {
  sessionId: number;
  onClose: () => void;
};

export function TimelineSessionDetail({ sessionId, onClose }: TimelineSessionDetailProps) {
  const t = useT();
  const locale = useLocale();
  const [detail, setDetail] = useState<TimelineSessionDetailDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const scrollFade = useScrollFade(contentRef);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError(null);

    void activityTimelineSession(sessionId)
      .then((res) => {
        if (!active) return;
        setDetail(res);
        setLoading(false);
      })
      .catch((err: unknown) => {
        if (!active) return;
        const msg =
          err && typeof err === "object" && "message" in err && typeof err.message === "string"
            ? err.message
            : "Could not load session details";
        setError(msg);
        setLoading(false);
      });

    return () => {
      active = false;
    };
  }, [sessionId]);

  const session = detail?.session;
  const categoryColor = session
    ? (CATEGORY_COLOR_VARS[session.category] ?? "var(--brand)")
    : "var(--brand)";
  const categoryLabel = session ? t(CATEGORY_LABEL_KEYS[session.category]) : "";
  const timeStr = session
    ? `${formatTime(session.startedAt, locale)} - ${formatTime(session.endedAt, locale)}`
    : "";
  const durationStr = session ? formatDuration(session.durationMs, locale) : "";

  return (
    <aside className="timeline-detail-panel" aria-label={t("activity.detail.windows")}>
      <div className="timeline-detail-header">
        <div className="timeline-detail-header-info">
          {session ? (
            <>
              <div className="timeline-detail-title-line">
                <span className="timeline-detail-app">{session.appName}</span>
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
              </div>
              <div className="timeline-detail-meta">
                <span>{timeStr}</span>
                <span aria-hidden="true">•</span>
                <span>{durationStr}</span>
              </div>
            </>
          ) : (
            <div className="timeline-detail-title-line">
              <span className="timeline-detail-app">{t("activity.timeline.title")}</span>
            </div>
          )}
        </div>
        <button
          type="button"
          className="timeline-detail-close-btn"
          onClick={onClose}
          aria-label={t("activity.detail.close")}
        >
          <IconCrossSmall size={16} />
        </button>
      </div>

      <div
        ref={contentRef}
        className="timeline-detail-content scroll-fade-mask"
        {...scrollFade.props}
      >
        {loading ? (
          <div className="timeline-detail-loading">
            <span className="dot-spinner" aria-hidden="true" />
          </div>
        ) : error ? (
          <div className="timeline-inline-error" role="alert">
            <p>{error}</p>
          </div>
        ) : detail ? (
          <>
            {/* Windows List */}
            <section className="timeline-detail-section">
              <h4 className="timeline-detail-section-title">{t("activity.detail.windows")}</h4>
              <div className="timeline-detail-windows-list">
                {detail.windows.map((win) => {
                  const winTime = `${formatTime(win.firstSeenAt, locale)} - ${formatTime(win.lastSeenAt, locale)}`;
                  const frameStr = t("activity.detail.frameCount", { count: win.frameCount });

                  return (
                    <div
                      key={`win-${win.windowTitle ?? win.browserUrl ?? "item"}-${win.firstSeenAt}`}
                      className="timeline-detail-window-card"
                    >
                      <div className="timeline-detail-window-header">
                        <span className="timeline-detail-window-icon" aria-hidden="true">
                          {win.browserUrl ? (
                            <IconGlobe size={14} />
                          ) : (
                            <IconWindowSparkle size={14} />
                          )}
                        </span>
                        <span className="timeline-detail-window-title">
                          {win.windowTitle || win.browserUrl || session?.appName}
                        </span>
                      </div>
                      {win.browserUrl ? (
                        <div className="timeline-detail-window-url" title={win.browserUrl}>
                          {win.browserUrl}
                        </div>
                      ) : null}
                      <div className="timeline-detail-window-meta">
                        <span>{winTime}</span>
                        <span aria-hidden="true">•</span>
                        <span>{frameStr}</span>
                      </div>
                    </div>
                  );
                })}
              </div>
            </section>

            {/* Text Excerpt */}
            {detail.textExcerpt ? (
              <section className="timeline-detail-section">
                <h4 className="timeline-detail-section-title">{t("activity.detail.excerpt")}</h4>
                <div className="timeline-detail-excerpt-card">
                  <pre className="timeline-detail-excerpt-text">{detail.textExcerpt}</pre>
                </div>
              </section>
            ) : null}
          </>
        ) : null}
      </div>
    </aside>
  );
}
