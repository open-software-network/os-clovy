import {
  type CSSProperties,
  type MouseEvent,
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { type InterfaceLocale, intlLocale, useLocale, useT } from "../../i18n";
import { useScrollFade } from "../../lib/use-scroll-fade";
import { formatDuration, formatTime } from "./timeline-utils";
import {
  ESTIMATED_HEADER_PX,
  type TimelineCard,
  visibleNotes,
  type WorkstreamTimelineLayout,
} from "./workstream-timeline-layout";

type WorkstreamTimelineProps = {
  layout: WorkstreamTimelineLayout;
  /** Today's current time, for the now line; null on other days. */
  nowMs: number | null;
  selectedId: number | null;
  onSelect: (id: number) => void;
  /** A click on the empty column drops the selection. */
  onClearSelection: () => void;
  /** The interface text-size preference (1 at the default size). */
  textScale?: number;
  viewportRef?: (element: HTMLDivElement | null) => void;
};

/** "2 PM" / "14h": an hour known only by its whole local hour. */
function formatHourOnly(ms: number, locale: InterfaceLocale): string {
  const date = new Date(ms);
  if (locale === "pt-BR") return `${date.getHours()}h`;
  return new Intl.DateTimeFormat(intlLocale(locale), { hour: "numeric" }).format(date);
}

/** A rail label from the local hour (0 to 23): "9 AM" / "9h". */
function formatRailHour(hour: number, locale: InterfaceLocale): string {
  if (locale === "pt-BR") return `${hour}h`;
  return `${hour % 12 || 12} ${hour < 12 ? "AM" : "PM"}`;
}

/**
 * Start and end of a span for "{start} - {end}". A span with any record known
 * only by hour shows whole hours, never minutes that were not measured.
 */
export function spanLabels(
  span: { startMs: number; endMs: number; approximate: boolean },
  locale: InterfaceLocale,
): { start: string; end: string } {
  if (span.approximate) {
    return {
      start: formatHourOnly(span.startMs, locale),
      end: formatHourOnly(span.endMs, locale),
    };
  }
  return {
    start: formatTime(new Date(span.startMs).toISOString(), locale),
    end: formatTime(new Date(span.endMs).toISOString(), locale),
  };
}

/**
 * The day's workstreams as a column of cards on a rail of real local hours.
 * A workstream has one card per stretch of work between long pauses; a card
 * spans its first to last sitting there, colored bars on its left edge mark
 * each sitting, and what it shows grows with its drawn height (title, then
 * the active time and range, then what was done). The active time printed on
 * it comes from the data, never from its height. Overlapping workstreams
 * share the width in columns.
 */
export function WorkstreamTimeline({
  layout,
  nowMs,
  selectedId,
  onSelect,
  onClearSelection,
  textScale = 1,
  viewportRef,
}: WorkstreamTimelineProps) {
  const t = useT();
  const locale = useLocale();
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const setScroller = useCallback(
    (element: HTMLDivElement | null) => {
      scrollerRef.current = element;
      viewportRef?.(element);
    },
    [viewportRef],
  );
  const fade = useScrollFade(scrollerRef);
  // The header counts workstreams, not cards: a workstream split by long
  // pauses is still one, and its minutes are counted once.
  const activeMs = layout.workstreams.reduce((total, span) => total + span.activeMs, 0);
  const firstCardTop = layout.cards[0]?.top ?? 0;
  const firstCardStart = layout.cards[0]?.startMs;
  const openedStart = useRef<number | undefined>();

  // Opening a day starts at its first workstream, not at the top of the axis.
  useEffect(() => {
    if (openedStart.current === firstCardStart) return;
    openedStart.current = firstCardStart;
    scrollerRef.current?.scrollTo?.({ top: Math.max(0, firstCardTop - 24) });
  }, [firstCardStart, firstCardTop]);

  const clearOnEmpty = (event: MouseEvent<HTMLDivElement>) => {
    if (selectedId === null) return;
    if ((event.target as Element).closest(".ws-timeline-card")) return;
    onClearSelection();
  };

  return (
    <section
      className="ws-timeline"
      aria-label={t("activity.grid.label")}
      data-has-selection={selectedId !== null || undefined}
    >
      {/* An empty grid sits under the notice that says why; it counts nothing. */}
      {layout.workstreams.length > 0 ? (
        <header className="ws-timeline-header">
          <h3 className="ws-timeline-title">
            {t("activity.grid.count", { count: layout.workstreams.length })}
          </h3>
          <span className="ws-timeline-total">
            {t("activity.grid.active", { duration: formatDuration(activeMs, locale) })}
          </span>
        </header>
      ) : null}
      <div className="ws-timeline-scroller scroll-fade-mask" ref={setScroller} {...fade.props}>
        {/* biome-ignore lint/a11y/useKeyWithClickEvents: Escape clears the selection from the keyboard. */}
        {/* biome-ignore lint/a11y/noStaticElementInteractions: the click only clears the selection; cards stay the controls. */}
        <div
          className="ws-timeline-canvas"
          data-testid="timeline-canvas"
          style={{ height: layout.height }}
          onClick={clearOnEmpty}
        >
          <div className="ws-timeline-spine" aria-hidden="true" />
          {layout.hours.map((hour) => (
            <div
              key={hour.ms}
              className="ws-timeline-hour"
              data-testid="timeline-hour"
              data-current={hour.current || undefined}
              style={{ top: hour.top }}
              aria-hidden="true"
            >
              <span className="ws-timeline-hour-label">{formatRailHour(hour.hour, locale)}</span>
              <span className="ws-timeline-hour-dot" />
              <span className="ws-timeline-hour-line" />
            </div>
          ))}
          <div className="ws-timeline-cards">
            {nowMs !== null ? (
              <div
                className="ws-timeline-now"
                style={{ top: layout.toPx(nowMs) }}
                title={t("activity.grid.now")}
                aria-hidden="true"
              />
            ) : null}
            {layout.cards.map((card) => (
              <WorkstreamCard
                key={card.key}
                card={card}
                selected={card.workstream.id === selectedId}
                textScale={textScale}
                onSelect={onSelect}
              />
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}

/** Memoized: the parent re-renders on every now tick and selection change. */
const WorkstreamCard = memo(function WorkstreamCard({
  card,
  selected,
  textScale,
  onSelect,
}: {
  card: TimelineCard;
  selected: boolean;
  textScale: number;
  onSelect: (id: number) => void;
}) {
  const t = useT();
  const locale = useLocale();
  const { workstream, density } = card;
  const range = t("activity.detail.timeRange", spanLabels(card, locale));
  const active = formatDuration(card.activeMs, locale);
  const sittings =
    !card.approximate && card.sittings > 1
      ? t("activity.grid.sittings", { count: card.sittings })
      : null;
  const { notes } = card;
  const headerRef = useRef<HTMLSpanElement | null>(null);
  const headerHeight = useHeaderHeight(headerRef, density === "notes");
  const { shown, more } = visibleNotes(
    notes.length,
    card.height,
    headerHeight || ESTIMATED_HEADER_PX * textScale,
    textScale,
  );

  return (
    <button
      type="button"
      className="ws-timeline-card"
      data-density={density}
      data-lane={card.lane}
      data-lanes={card.laneCount}
      aria-pressed={selected}
      aria-label={[
        workstream.title,
        t("activity.grid.active", { duration: active }),
        range,
        card.approximate ? t("activity.grid.byHour") : null,
        sittings,
      ]
        .filter(Boolean)
        .join(", ")}
      onClick={() => onSelect(workstream.id)}
      style={
        {
          top: card.top,
          height: card.height,
          "--lane": card.lane,
          "--lanes": card.laneCount,
        } as CSSProperties
      }
    >
      <span className="ws-timeline-card-rail" aria-hidden="true">
        {card.bars.map((bar, index) => (
          <span
            key={`${card.segments[index].kind}-${card.segments[index].startMs}`}
            className="ws-timeline-sitting"
            data-approximate={bar.approximate || undefined}
            data-testid={bar.approximate ? undefined : "workstream-sitting"}
            style={{ top: bar.top, height: bar.height }}
          />
        ))}
      </span>
      {density === "pill" ? (
        <span className="ws-timeline-card-title">{workstream.title}</span>
      ) : (
        <span className="ws-timeline-card-body">
          <span className="ws-timeline-card-header" ref={headerRef}>
            <span className="ws-timeline-card-heading">
              <span className="ws-timeline-card-dot" data-testid="card-dot" />
              <span className="ws-timeline-card-title">{workstream.title}</span>
            </span>
            {density === "title" ? null : (
              <span className="ws-timeline-card-meta">
                <span className="ws-timeline-card-active" data-testid="card-duration">
                  {active}
                </span>
                {card.approximate ? (
                  <span className="ws-timeline-badge" title={t("activity.grid.byHourHint")}>
                    {t("activity.grid.byHour")}
                  </span>
                ) : null}
                <span className="ws-timeline-card-range">{range}</span>
                {sittings ? <span className="ws-timeline-card-sittings">· {sittings}</span> : null}
              </span>
            )}
          </span>
          {density === "notes" && shown > 0 ? (
            <>
              <span className="ws-timeline-card-notes">
                {notes.slice(0, shown).map((note, index) => (
                  <span
                    // biome-ignore lint/suspicious/noArrayIndexKey: notes are plain text in hour order, not entities.
                    key={index}
                    className="ws-timeline-card-note"
                    data-testid="card-note"
                  >
                    {note}
                  </span>
                ))}
              </span>
              {more > 0 ? (
                <span className="ws-timeline-card-more" data-testid="card-more">
                  {t("activity.grid.moreNotes", { count: more })}
                </span>
              ) : null}
            </>
          ) : null}
        </span>
      )}
    </button>
  );
});

/** The rendered height of a card's title and meta, 0 until measured. */
function useHeaderHeight(ref: { current: HTMLSpanElement | null }, enabled: boolean) {
  const [height, setHeight] = useState(0);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!enabled || !element) return;
    const measure = () => setHeight(Math.round(element.getBoundingClientRect().height));
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref, enabled]);
  return height;
}
