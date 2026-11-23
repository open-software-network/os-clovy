import { useEffect, useRef, useState } from "react";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconGlobe } from "central-icons/IconGlobe";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { IconWindowSparkle } from "central-icons/IconWindowSparkle";
import { intlLocale, useLocale, useT } from "../../i18n";
import {
  activityTimelineSearch,
  dayRange,
  type TimelineSearchResultDto,
} from "../../lib/activity-timeline";
import { useScrollFade } from "../../lib/use-scroll-fade";
import { formatTime } from "./timeline-utils";

type SearchPeriod = "today" | "7d" | "30d" | "all";

type TimelineSearchProps = {
  onSelectResult: (result: TimelineSearchResultDto) => void;
};

export function TimelineSearch({ onSelectResult }: TimelineSearchProps) {
  const t = useT();
  const locale = useLocale();
  const [query, setQuery] = useState("");
  const [period, setPeriod] = useState<SearchPeriod>("today");
  const [results, setResults] = useState<TimelineSearchResultDto[]>([]);
  const [loading, setLoading] = useState(false);
  const [hasSearched, setHasSearched] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The results popover closes once a result is opened and reopens on input.
  const [open, setOpen] = useState(true);

  const resultsRef = useRef<HTMLDivElement>(null);
  const scrollFade = useScrollFade(resultsRef);

  useEffect(() => {
    const trimmed = query.trim();
    if (!trimmed) {
      setResults([]);
      setLoading(false);
      setHasSearched(false);
      setError(null);
      return;
    }

    let active = true;
    setLoading(true);
    setError(null);

    const timer = setTimeout(() => {
      let from: string | null = null;
      let to: string | null = null;

      const now = new Date();
      if (period === "today") {
        const range = dayRange(now);
        from = range.from;
        to = range.to;
      } else if (period === "7d") {
        const d = new Date(now.getTime() - 7 * 86_400_000);
        from = d.toISOString();
      } else if (period === "30d") {
        const d = new Date(now.getTime() - 30 * 86_400_000);
        from = d.toISOString();
      }

      void activityTimelineSearch({
        query: trimmed,
        from,
        to,
        limit: 50,
      })
        .then((res) => {
          if (!active) return;
          setResults(res.results);
          setLoading(false);
          setHasSearched(true);
        })
        .catch((err: unknown) => {
          if (!active) return;
          const msg =
            err && typeof err === "object" && "message" in err && typeof err.message === "string"
              ? err.message
              : t("activity.search.failed");
          setError(msg);
          setLoading(false);
          setHasSearched(true);
        });
    }, 250);

    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [query, period, t]);

  return (
    <div className="timeline-search-bar">
      <div className="timeline-search-input-wrapper">
        <IconMagnifyingGlass size={15} className="timeline-search-icon" aria-hidden="true" />
        <input
          type="search"
          className="timeline-search-input"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          placeholder={t("activity.search.placeholder")}
          aria-label={t("activity.search.placeholder")}
        />
        {query.length > 0 ? (
          <button
            type="button"
            className="timeline-search-clear-btn"
            onClick={() => setQuery("")}
            aria-label={t("activity.search.clear")}
          >
            <IconCrossSmall size={14} />
          </button>
        ) : null}
      </div>

      <div className="timeline-search-periods">
        <button
          type="button"
          className="timeline-period-pill"
          data-active={period === "today" || undefined}
          onClick={() => setPeriod("today")}
          aria-pressed={period === "today"}
        >
          {t("activity.search.today")}
        </button>
        <button
          type="button"
          className="timeline-period-pill"
          data-active={period === "7d" || undefined}
          onClick={() => setPeriod("7d")}
          aria-pressed={period === "7d"}
        >
          {t("activity.search.last7Days")}
        </button>
        <button
          type="button"
          className="timeline-period-pill"
          data-active={period === "30d" || undefined}
          onClick={() => setPeriod("30d")}
          aria-pressed={period === "30d"}
        >
          {t("activity.search.last30Days")}
        </button>
        <button
          type="button"
          className="timeline-period-pill"
          data-active={period === "all" || undefined}
          onClick={() => setPeriod("all")}
          aria-pressed={period === "all"}
        >
          {t("activity.search.all")}
        </button>
      </div>

      {/* Floating or embedded Search Results Dropdown when query is non-empty */}
      {open && query.trim().length > 0 ? (
        <div
          ref={resultsRef}
          className="timeline-search-results-popover scroll-fade-mask"
          {...scrollFade.props}
        >
          {loading ? (
            <div className="timeline-search-loading">
              <span className="dot-spinner" aria-hidden="true" />
              <span>{t("activity.search.searching")}</span>
            </div>
          ) : error ? (
            <div className="timeline-search-error" role="alert">
              <p>{error}</p>
            </div>
          ) : results.length === 0 && hasSearched ? (
            <div className="timeline-search-empty">
              <p>{t("activity.search.noResults", { query: query.trim() })}</p>
            </div>
          ) : (
            results.map((res) => {
              const seenDate = new Date(res.seenAt);
              const formattedDate = new Intl.DateTimeFormat(intlLocale(locale), {
                month: "short",
                day: "numeric",
              }).format(seenDate);
              const timeStr = `${formattedDate}, ${formatTime(res.seenAt, locale)}`;

              return (
                <button
                  key={`search-res-${res.sessionId}-${res.seenAt}`}
                  type="button"
                  className="timeline-search-result-item"
                  onClick={() => {
                    setOpen(false);
                    onSelectResult(res);
                  }}
                >
                  <div className="timeline-search-result-header">
                    <span className="timeline-search-result-app">{res.appName}</span>
                    <span className="timeline-search-result-time">{timeStr}</span>
                  </div>
                  <div className="timeline-search-result-title">
                    {res.browserUrl ? (
                      <span className="timeline-search-url-icon" aria-hidden="true">
                        <IconGlobe size={13} />
                      </span>
                    ) : (
                      <span className="timeline-search-window-icon" aria-hidden="true">
                        <IconWindowSparkle size={13} />
                      </span>
                    )}
                    <span>{res.windowTitle || res.browserUrl || res.appName}</span>
                  </div>
                  {res.snippet ? (
                    <div className="timeline-search-result-snippet">
                      <p>{res.snippet}</p>
                    </div>
                  ) : null}
                </button>
              );
            })
          )}
        </div>
      ) : null}
    </div>
  );
}
