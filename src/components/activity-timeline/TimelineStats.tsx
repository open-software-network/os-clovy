import { useLocale, useT } from "../../i18n";
import type { TimelineStatsDto } from "../../lib/activity-timeline";
import { AppIcon } from "./AppIcon";
import { CATEGORY_COLOR_VARS, CATEGORY_LABEL_KEYS, formatDuration } from "./timeline-utils";

type TimelineStatsProps = {
  stats: TimelineStatsDto;
};

export function TimelineStats({ stats }: TimelineStatsProps) {
  const t = useT();
  const locale = useLocale();

  const totalTrackedMs = Math.max(1, stats.focusedMs + stats.idleMs + stats.awayMs);
  const maxAppMs = Math.max(1, ...stats.topApps.map((a) => a.durationMs));

  return (
    <aside className="timeline-stats-panel" aria-label={t("activity.stats.categories")}>
      {/* 3 Metric Cards */}
      <div className="timeline-stats-metrics">
        <div className="timeline-metric-card" data-metric="focused">
          <span className="timeline-metric-label">{t("activity.stats.focused")}</span>
          <span className="timeline-metric-value">{formatDuration(stats.focusedMs, locale)}</span>
          <div className="timeline-metric-bar-track">
            <div
              className="timeline-metric-bar-fill"
              style={{
                width: `${(stats.focusedMs / totalTrackedMs) * 100}%`,
                backgroundColor: "var(--brand)",
              }}
            />
          </div>
        </div>

        <div className="timeline-metric-card" data-metric="idle">
          <span className="timeline-metric-label">{t("activity.stats.idle")}</span>
          <span className="timeline-metric-value">{formatDuration(stats.idleMs, locale)}</span>
          <div className="timeline-metric-bar-track">
            <div
              className="timeline-metric-bar-fill"
              style={{
                width: `${(stats.idleMs / totalTrackedMs) * 100}%`,
                backgroundColor: "var(--muted-foreground)",
              }}
            />
          </div>
        </div>

        <div className="timeline-metric-card" data-metric="away">
          <span className="timeline-metric-label">{t("activity.stats.away")}</span>
          <span className="timeline-metric-value">{formatDuration(stats.awayMs, locale)}</span>
          <div className="timeline-metric-bar-track">
            <div
              className="timeline-metric-bar-fill"
              style={{
                width: `${(stats.awayMs / totalTrackedMs) * 100}%`,
                backgroundColor: "var(--border)",
              }}
            />
          </div>
        </div>
      </div>

      {/* Category Distribution */}
      {stats.categories.length > 0 ? (
        <section className="timeline-stats-section">
          <h4 className="timeline-stats-heading">{t("activity.stats.categories")}</h4>
          {/* Stacked distribution bar */}
          <div className="timeline-category-stacked-bar">
            {stats.categories.map((cat) => {
              const widthPct = stats.focusedMs > 0 ? (cat.durationMs / stats.focusedMs) * 100 : 0;
              const color = CATEGORY_COLOR_VARS[cat.category] ?? "var(--brand)";
              const label = t(CATEGORY_LABEL_KEYS[cat.category]);

              return (
                <div
                  key={cat.category}
                  className="timeline-stacked-segment"
                  style={{
                    width: `${widthPct}%`,
                    backgroundColor: color,
                  }}
                  title={`${label}: ${formatDuration(cat.durationMs, locale)} (${Math.round(widthPct)}%)`}
                />
              );
            })}
          </div>

          {/* Category List */}
          <div className="timeline-categories-list">
            {stats.categories.map((cat) => {
              const color = CATEGORY_COLOR_VARS[cat.category] ?? "var(--brand)";
              const label = t(CATEGORY_LABEL_KEYS[cat.category]);
              const pct =
                stats.focusedMs > 0 ? Math.round((cat.durationMs / stats.focusedMs) * 100) : 0;

              return (
                <div key={cat.category} className="timeline-category-row">
                  <div className="timeline-category-row-header">
                    <span
                      className="timeline-category-dot"
                      style={{ backgroundColor: color }}
                      aria-hidden="true"
                    />
                    <span className="timeline-category-row-name">{label}</span>
                    <span className="timeline-category-row-time">
                      {formatDuration(cat.durationMs, locale)}
                    </span>
                    <span className="timeline-category-row-pct">{pct}%</span>
                  </div>
                  <div className="timeline-category-bar-track">
                    <div
                      className="timeline-category-bar-fill"
                      style={{
                        width: `${pct}%`,
                        backgroundColor: color,
                      }}
                    />
                  </div>
                </div>
              );
            })}
          </div>
        </section>
      ) : null}

      {/* Top Apps */}
      {stats.topApps.length > 0 ? (
        <section className="timeline-stats-section">
          <h4 className="timeline-stats-heading" id="timeline-top-apps-heading">
            {t("activity.stats.topApps")}
          </h4>
          <ul className="timeline-top-apps-list" aria-labelledby="timeline-top-apps-heading">
            {stats.topApps.map((app) => {
              const pctOfMax = Math.round((app.durationMs / maxAppMs) * 100);

              return (
                <li key={app.bundleId ?? `name:${app.appName}`} className="timeline-top-app-row">
                  <AppIcon bundleId={app.bundleId} size={20} />
                  <div className="timeline-top-app-body">
                    <div className="timeline-top-app-header">
                      <span className="timeline-top-app-name">{app.appName}</span>
                      <span className="timeline-top-app-time">
                        {formatDuration(app.durationMs, locale)}
                      </span>
                    </div>
                    <div className="timeline-top-app-track">
                      <div className="timeline-top-app-fill" style={{ width: `${pctOfMax}%` }} />
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>
        </section>
      ) : null}
    </aside>
  );
}
