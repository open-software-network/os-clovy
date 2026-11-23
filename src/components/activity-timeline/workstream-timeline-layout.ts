import type { WorkstreamDto } from "../../lib/day-intelligence";
import { type LocalHour, localHourStarts } from "./timeline-utils";

/**
 * Pure layout of the Today → Timeline → Workstreams column: workstreams as
 * cards on a vertical axis of the day's real local hours.
 *
 * - A workstream is one card per stretch of work: a pause longer than
 *   `CARD_SPLIT_GAP_MS` starts a new card of the same workstream at its real
 *   time; shorter pauses stay gaps between the sitting bars of one card. A
 *   card's active time is the located seconds of the periods inside it
 *   plus the unlocated seconds of the by-hour parts it holds.
 *
 * - The window runs from 20 minutes before the first sitting to 20 minutes
 *   after the last one (and to now, today), at least 150 minutes centered on
 *   the work.
 * - An hour with any work is drawn whole; a run of consecutive hours without
 *   work takes at most the height of one hour.
 * - The base scale is 1 px per minute; a day shorter than the canvas grows by
 *   one factor (capped at 9 px per minute) until it fills it.
 * - Positions come from the bounds of the measured sessions a workstream's
 *   work was located in (its sittings), never from its active time. Work
 *   known only by hour (a legacy record, or the seconds of an hour no session
 *   was named for) spans its whole local hour and is flagged approximate.
 * - DST days keep their 23 or 25 real local hours.
 */

/**
 * One stretch of a card: `session` is where located work happened (bounds of
 * measured sessions, joined); `hour` is a legacy hour known only by hour;
 * `unlocated` is the part of an hour's seconds no session was named for.
 */
export type TimelineSegment = {
  startMs: number;
  endMs: number;
  kind: "session" | "hour" | "unlocated";
  /** Known only by hour (`hour` or `unlocated`). */
  approximate: boolean;
  /** For an approximate segment: its active minutes in that hour. */
  minutes: number | null;
  /** For an approximate segment: its active seconds in that hour. */
  seconds?: number | null;
};

/** A sitting bar on the card's left rail, in the card's padding box. */
export type TimelineBar = { top: number; height: number; approximate: boolean };

/** What a card shows at its drawn height (spec section 5). */
export type CardDensity = "pill" | "title" | "meta" | "notes";

/** A stretch of a workstream's day, or the whole of it. */
export type TimelineSpan = {
  workstream: WorkstreamDto;
  /** Ascending by start; session bounds already joined. */
  segments: TimelineSegment[];
  startMs: number;
  endMs: number;
  /** Some of its work is known only by hour. */
  approximate: boolean;
  /** Separate session stretches (resumptions), not counting hour-only parts. */
  sittings: number;
  /** Active time from the data, independent of the span. */
  activeMs: number;
};

/** One card: a stretch of a workstream between pauses longer than the split gap. */
export type TimelineCard = TimelineSpan & {
  /** Unique per card: the workstream id and the stretch's index. */
  key: string;
  /** Index of the stretch within its workstream's day. */
  part: number;
  /** Notes of the hours whose work falls in this stretch, in hour order. */
  notes: string[];
  lane: number;
  laneCount: number;
  top: number;
  height: number;
  bars: TimelineBar[];
  density: CardDensity;
};

/** A labeled hour on the rail: `hour` is its local hour, 0 to 23. */
export type TimelineHour = { ms: number; hour: number; top: number; current: boolean };

export type WorkstreamTimelineLayout = {
  /** Each workstream whole, in card order: what the header and details show. */
  workstreams: TimelineSpan[];
  cards: TimelineCard[];
  hours: TimelineHour[];
  height: number;
  pxPerMin: number;
  toPx: (ms: number) => number;
};

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
/** Breathing room before the first and after the last sitting. */
const WINDOW_PAD_MS = 20 * MINUTE_MS;
const MIN_WINDOW_MS = 150 * MINUTE_MS;
const BASE_PX_PER_MIN = 1;
const MAX_PX_PER_MIN = 9;
/**
 * A pause longer than this between a workstream's sittings starts a new card
 * of the same workstream; shorter pauses stay inside one card.
 */
export const CARD_SPLIT_GAP_MS = 45 * MINUTE_MS;
/** Measured periods closer than this read as one sitting (second rounding). */
const SITTING_JOIN_MS = MINUTE_MS;
/** Card border, so bars stay inside the padding box. */
const CARD_BORDER_PX = 1;
const BAR_INSET_PX = 10;
const BAR_INSET_SHARE = 0.25;
const BAR_MIN_PX = 4;
/** Drawn-height thresholds at the default text size (spec section 5). */
const DENSITY_PX = { title: 46, meta: 62, notes: 104 } as const;
/** Note slots: header measured, then 8 px, then 17 px per slot. */
const NOTES_GAP_PX = 8;
const NOTE_SLOT_PX = 17;
/** Header height (title and meta, one title line) before it is measured. */
export const ESTIMATED_HEADER_PX = 38;

/** The local window of an hour report key: first start to the next other key. */
function hourWindow(
  hours: LocalHour[],
  key: string,
  dayEndMs: number,
): { startMs: number; endMs: number } | null {
  const index = hours.findIndex((local) => local.key === key);
  if (index < 0) return null;
  const next = hours.slice(index + 1).find((local) => local.key !== key);
  return { startMs: hours[index].ms, endMs: next?.ms ?? dayEndMs };
}

/** Where one hour report's work sits: its measured periods and its by-hour part. */
type HourPlacement = {
  located: { startMs: number; endMs: number }[];
  byHour: TimelineSegment | null;
  /** The hour's own local window, for an hour with no position of its own. */
  window: { startMs: number; endMs: number } | null;
};

function hourPlacements(
  workstream: WorkstreamDto,
  hours: LocalHour[],
  dayStartMs: number,
  dayEndMs: number,
): HourPlacement[] {
  return workstream.hours.map((hour) => {
    const hourSeconds = hour.seconds ?? hour.minutes * 60;
    const locatedSeconds = hour.locatedSeconds ?? (hour.periods ? hour.locatedMinutes * 60 : 0);
    const unlocatedSeconds = hour.periods ? hourSeconds - locatedSeconds : hourSeconds;
    const unlocatedMinutes = hour.periods ? hour.minutes - hour.locatedMinutes : hour.minutes;
    const located: HourPlacement["located"] = [];
    for (const period of hour.periods ?? []) {
      const startMs = Math.max(Date.parse(period.startedAt), dayStartMs);
      const endMs = Math.min(Date.parse(period.endedAt), dayEndMs);
      if (endMs > startMs) located.push({ startMs, endMs });
    }
    const window = hourWindow(hours, hour.hour, dayEndMs);
    const byHour: TimelineSegment | null =
      window && !(hour.periods && unlocatedSeconds <= 0)
        ? {
            ...window,
            kind: hour.periods ? "unlocated" : "hour",
            approximate: true,
            minutes: unlocatedMinutes,
            seconds: unlocatedSeconds,
          }
        : null;
    return { located, byHour, window };
  });
}

/** A workstream's segments: located periods joined into sittings, plus by-hour parts. */
function joinSegments(placements: HourPlacement[]): TimelineSegment[] {
  const located = placements.flatMap((placement) => placement.located);
  located.sort((a, b) => a.startMs - b.startMs);
  const sessions: TimelineSegment[] = [];
  for (const period of located) {
    const last = sessions[sessions.length - 1];
    if (last && period.startMs - last.endMs <= SITTING_JOIN_MS) {
      last.endMs = Math.max(last.endMs, period.endMs);
    } else {
      sessions.push({
        ...period,
        kind: "session",
        approximate: false,
        minutes: null,
        seconds: null,
      });
    }
  }
  const byHour = placements.flatMap((placement) => (placement.byHour ? [placement.byHour] : []));
  return [...sessions, ...byHour].sort((a, b) => a.startMs - b.startMs || a.endMs - b.endMs);
}

/** Splits ascending segments where the pause since the work so far exceeds the gap. */
function splitByPause(segments: TimelineSegment[]): TimelineSegment[][] {
  const stretches: TimelineSegment[][] = [];
  let reachedMs = Number.NEGATIVE_INFINITY;
  for (const segment of segments) {
    if (segment.startMs - reachedMs > CARD_SPLIT_GAP_MS) stretches.push([]);
    stretches[stretches.length - 1].push(segment);
    reachedMs = Math.max(reachedMs, segment.endMs);
  }
  return stretches;
}

/** Time of `ranges` that falls inside a stretch. */
function overlapWith(stretch: TimelineSpan, ranges: { startMs: number; endMs: number }[]): number {
  return ranges.reduce(
    (sum, range) =>
      sum +
      Math.max(0, Math.min(stretch.endMs, range.endMs) - Math.max(stretch.startMs, range.startMs)),
    0,
  );
}

/**
 * The stretch with most of an hour report's placed time (its periods and
 * by-hour part, or else its local window); the first one when it has none.
 * The hour's note goes there.
 */
function stretchOfHour(placement: HourPlacement, stretches: TimelineSpan[]): number {
  const placed = [...placement.located, ...(placement.byHour ? [placement.byHour] : [])];
  const own = placed.length > 0 ? placed : placement.window ? [placement.window] : [];
  let best = 0;
  let bestOverlap = 0;
  stretches.forEach((stretch, index) => {
    const overlap = overlapWith(stretch, own);
    if (overlap > bestOverlap) {
      best = index;
      bestOverlap = overlap;
    }
  });
  return best;
}

/**
 * Whole seconds split by weight, largest remainder first, so the shares
 * always add up to `seconds`.
 */
function shareSeconds(seconds: number, weights: number[]): number[] {
  const total = weights.reduce((sum, weight) => sum + weight, 0);
  if (total <= 0 || seconds <= 0) return weights.map(() => 0);
  const exact = weights.map((weight) => (seconds * weight) / total);
  const shares = exact.map(Math.floor);
  let left = seconds - shares.reduce((sum, share) => sum + share, 0);
  const order = exact
    .map((value, index) => ({ index, fraction: value - Math.floor(value) }))
    .sort((a, b) => b.fraction - a.fraction || a.index - b.index);
  for (const { index } of order) {
    if (left <= 0) break;
    shares[index] += 1;
    left -= 1;
  }
  return shares;
}

/**
 * Active seconds an hour report adds to each stretch. Its located seconds go
 * to the stretches its measured periods fall in, in proportion to them; the
 * seconds no period located go to the stretch holding its by-hour part, which
 * is one segment of the cut, so that stretch is the one drawn approximate.
 */
function hourSecondsByStretch(
  hour: WorkstreamDto["hours"][number],
  placement: HourPlacement,
  stretches: TimelineSpan[],
): number[] {
  const added = stretches.map(() => 0);
  const hourSeconds = hour.seconds ?? hour.minutes * 60;
  const locatedSeconds = hour.locatedSeconds ?? (hour.periods ? hour.locatedMinutes * 60 : 0);
  const located = hour.periods ? locatedSeconds : 0;
  const weights = stretches.map((stretch) => overlapWith(stretch, placement.located));
  if (located !== 0 && weights.some((weight) => weight > 0)) {
    shareSeconds(located, weights).forEach((share, index) => {
      added[index] += share;
    });
  } else {
    added[stretchOfHour(placement, stretches)] += located;
  }
  const unlocated = hourSeconds - located;
  if (unlocated !== 0) {
    const byHour = placement.byHour;
    const home = byHour ? stretches.findIndex((stretch) => overlapWith(stretch, [byHour]) > 0) : -1;
    added[home >= 0 ? home : stretchOfHour(placement, stretches)] += unlocated;
  }
  return added;
}

function spanOf(
  workstream: WorkstreamDto,
  segments: TimelineSegment[],
  activeMs: number,
): TimelineSpan {
  return {
    workstream,
    segments,
    startMs: segments[0].startMs,
    endMs: Math.max(...segments.map((segment) => segment.endMs)),
    approximate: segments.some((segment) => segment.approximate),
    sittings: segments.filter((segment) => !segment.approximate).length,
    activeMs,
  };
}

/** A workstream whole, and its stretches with the seconds and notes of their hours. */
function placeWorkstream(
  workstream: WorkstreamDto,
  hours: LocalHour[],
  dayStartMs: number,
  dayEndMs: number,
): { whole: TimelineSpan; stretches: (TimelineSpan & { notes: string[] })[] } | null {
  const placements = hourPlacements(workstream, hours, dayStartMs, dayEndMs);
  const segments = joinSegments(placements);
  if (segments.length === 0) return null;
  const stretches = splitByPause(segments).map((part) => ({
    ...spanOf(workstream, part, 0),
    notes: [] as string[],
  }));
  const workstreamSeconds = workstream.seconds ?? workstream.minutes * 60;
  workstream.hours.forEach((hour, index) => {
    hourSecondsByStretch(hour, placements[index], stretches).forEach((seconds, part) => {
      stretches[part].activeMs += seconds * 1000;
    });
    if (hour.note) stretches[stretchOfHour(placements[index], stretches)].notes.push(hour.note);
  });
  return { whole: spanOf(workstream, segments, workstreamSeconds * 1000), stretches };
}

/** The visible stretch of the day, before any scale is applied. */
function timeWindow(
  firstMs: number,
  lastMs: number,
  nowMs: number | null,
  dayStartMs: number,
  dayEndMs: number,
): { startMs: number; endMs: number } {
  let startMs = firstMs - WINDOW_PAD_MS;
  let endMs = Math.max(lastMs + WINDOW_PAD_MS, nowMs ?? Number.NEGATIVE_INFINITY);
  if (endMs - startMs < MIN_WINDOW_MS) {
    const middle = (firstMs + lastMs) / 2;
    startMs = Math.floor((middle - MIN_WINDOW_MS / 2) / MINUTE_MS) * MINUTE_MS;
    endMs = Math.ceil((middle + MIN_WINDOW_MS / 2) / MINUTE_MS) * MINUTE_MS;
    // Centered on the work alone it could end before now, today.
    if (nowMs !== null) endMs = Math.max(endMs, nowMs);
  }
  if (startMs < dayStartMs) {
    endMs += dayStartMs - startMs;
    startMs = dayStartMs;
  }
  if (endMs > dayEndMs) {
    startMs = Math.max(dayStartMs, startMs - (endMs - dayEndMs));
    endMs = dayEndMs;
  }
  return { startMs, endMs };
}

/** A piece of the window: part of one local hour, or a run of idle hours. */
type Piece = { startMs: number; endMs: number; natural: number; top: number; height: number };

function buildPieces(
  hours: LocalHour[],
  dayEndMs: number,
  window: { startMs: number; endMs: number },
  busy: (startMs: number, endMs: number) => boolean,
): Piece[] {
  const pieces: (Piece & { active: boolean })[] = [];
  hours.forEach((hour, index) => {
    const hourEnd = hours[index + 1]?.ms ?? dayEndMs;
    const startMs = Math.max(hour.ms, window.startMs);
    const endMs = Math.min(hourEnd, window.endMs);
    if (endMs <= startMs) return;
    // One minute of work makes the whole hour active.
    const active = busy(hour.ms, hourEnd);
    const previous = pieces[pieces.length - 1];
    if (previous && !previous.active && !active) {
      previous.endMs = endMs;
      return;
    }
    pieces.push({ startMs, endMs, natural: 0, top: 0, height: 0, active });
  });
  for (const piece of pieces) {
    const minutes = (piece.endMs - piece.startMs) / MINUTE_MS;
    piece.natural = piece.active ? minutes : Math.min(minutes, HOUR_MS / MINUTE_MS);
  }
  return pieces;
}

/** A card before lanes, with its real (not minimum) height. */
type PlacedCard = TimelineCard & { realHeight: number };

/** Leftmost free lane per card, then the widest lane count of each group. */
function assignLanes(cards: PlacedCard[], minHeight: number, gap: number) {
  const sorted = [...cards].sort(
    (a, b) => a.top - b.top || b.height - a.height || a.workstream.id - b.workstream.id,
  );
  const laneEnds: number[] = [];
  let group: PlacedCard[] = [];
  let groupEnd = Number.NEGATIVE_INFINITY;
  const closeGroup = () => {
    const count = Math.max(1, ...group.map((card) => card.lane + 1));
    for (const card of group) card.laneCount = count;
    group = [];
    laneEnds.length = 0;
  };
  for (const card of sorted) {
    if (card.top >= groupEnd) closeGroup();
    // A card stretched to the minimum height also keeps the gap below it;
    // a real span lets the next card touch it.
    const end = card.top + Math.max(card.realHeight, minHeight + gap);
    let lane = laneEnds.findIndex((laneEnd) => laneEnd <= card.top);
    if (lane < 0) lane = laneEnds.length;
    laneEnds[lane] = end;
    card.lane = lane;
    group.push(card);
    groupEnd = Math.max(groupEnd, end);
  }
  closeGroup();
}

function barsFor(
  segments: TimelineSegment[],
  cardTop: number,
  cardHeight: number,
  toPx: (ms: number) => number,
): TimelineBar[] {
  const inset = Math.min(BAR_INSET_PX, cardHeight * BAR_INSET_SHARE);
  const innerHeight = cardHeight - 2 * CARD_BORDER_PX;
  return segments.map((segment) => {
    const top = Math.max(toPx(segment.startMs) - cardTop, inset);
    const bottom = Math.min(toPx(segment.endMs) - cardTop, cardHeight - inset);
    const height = Math.max(BAR_MIN_PX, bottom - top);
    // A sitting at the very end of a card stays inside it, never cut.
    return {
      top: Math.max(0, Math.min(top, innerHeight - height)),
      height,
      approximate: segment.approximate,
    };
  });
}

export function densityFor(height: number, textScale = 1): CardDensity {
  if (height >= DENSITY_PX.notes * textScale) return "notes";
  if (height >= DENSITY_PX.meta * textScale) return "meta";
  if (height >= DENSITY_PX.title * textScale) return "title";
  return "pill";
}

/**
 * How many notes a card shows: the slots below the measured header; when not
 * all fit, one slot goes to the "+N more" count, which is never shown alone.
 */
export function visibleNotes(
  count: number,
  cardHeight: number,
  headerHeight: number,
  textScale = 1,
): { shown: number; more: number } {
  const slots = Math.max(
    0,
    Math.floor((cardHeight - headerHeight - NOTES_GAP_PX) / (NOTE_SLOT_PX * textScale)),
  );
  if (count <= slots) return { shown: count, more: 0 };
  const shown = Math.max(0, slots - 1);
  return shown > 0 ? { shown, more: count - shown } : { shown: 0, more: 0 };
}

type Range = { startMs: number; endMs: number };

/**
 * The hour axis around some work: the visible window, its scale, and the rail
 * labels. Shared by the grid of workstreams and the empty grid of a day that
 * has none yet, so both follow the same window and scale rules.
 */
function layoutAxis(
  work: Range[],
  hours: LocalHour[],
  options: { dayStartMs: number; dayEndMs: number; nowMs: number | null; canvasHeight?: number },
) {
  const { dayStartMs, dayEndMs, nowMs } = options;
  const busy = (startMs: number, endMs: number) =>
    work.some((range) => range.startMs < endMs && range.endMs > startMs);
  const window = timeWindow(
    Math.min(...work.map((range) => range.startMs)),
    Math.max(...work.map((range) => range.endMs)),
    nowMs,
    dayStartMs,
    dayEndMs,
  );
  const pieces = buildPieces(hours, dayEndMs, window, busy);
  const natural = pieces.reduce((sum, piece) => sum + piece.natural, 0);
  const available = Number.isFinite(options.canvasHeight) ? (options.canvasHeight ?? 0) : 0;
  const pxPerMin =
    available > natural * BASE_PX_PER_MIN
      ? Math.min(MAX_PX_PER_MIN, available / natural)
      : BASE_PX_PER_MIN;
  let top = 0;
  for (const piece of pieces) {
    piece.top = top;
    piece.height = piece.natural * pxPerMin;
    top += piece.height;
  }
  const height = Math.round(top);
  const toPx = (ms: number) => {
    const clamped = Math.min(Math.max(ms, window.startMs), window.endMs);
    const piece =
      pieces.find((candidate) => clamped >= candidate.startMs && clamped < candidate.endMs) ??
      pieces[pieces.length - 1];
    const fraction = (clamped - piece.startMs) / (piece.endMs - piece.startMs);
    return Math.round(piece.top + fraction * piece.height);
  };

  // Labels: every hour with work and the hour right after one (where the work
  // stopped); today, nothing after the current hour, which is always marked.
  const marks: TimelineHour[] = [];
  hours.forEach((hour, index) => {
    const hourEnd = hours[index + 1]?.ms ?? dayEndMs;
    const current = nowMs !== null && hour.ms <= nowMs && nowMs < hourEnd;
    if (nowMs !== null && hour.ms > nowMs) return;
    const previous = hours[index - 1];
    const worked = busy(hour.ms, hourEnd);
    const workedBefore = previous !== undefined && busy(previous.ms, hour.ms);
    if (!worked && !workedBefore && !current) return;
    marks.push({ ms: hour.ms, hour: hour.hour, top: toPx(hour.ms), current });
  });

  return { hours: marks, height, pxPerMin, toPx };
}

/** Today's current time when it falls inside the day; null otherwise. */
function nowInDay(nowMs: number | null | undefined, dayStartMs: number, dayEndMs: number) {
  return nowMs != null && nowMs > dayStartMs && nowMs < dayEndMs ? nowMs : null;
}

const EMPTY_LAYOUT: WorkstreamTimelineLayout = {
  workstreams: [],
  cards: [],
  hours: [],
  height: 0,
  pxPerMin: BASE_PX_PER_MIN,
  toPx: () => 0,
};

export function layoutWorkstreamTimeline(
  workstreams: WorkstreamDto[],
  options: {
    dayStartMs: number;
    dayEndMs: number;
    /** Today's current time; null on other days. */
    nowMs?: number | null;
    timeZone?: string;
    /** Usable canvas height: the scroller's content box. */
    canvasHeight?: number;
    /** Resolved --control-xs / title line box and --sp-2 from the interface. */
    minCardHeight?: number;
    cardGap?: number;
    /** The interface text-size preference (1 at the default size). */
    textScale?: number;
  },
): WorkstreamTimelineLayout {
  const { dayStartMs, dayEndMs, timeZone } = options;
  const minHeight = options.minCardHeight ?? 22;
  const gap = options.cardGap ?? 6;
  const textScale = options.textScale ?? 1;
  const nowMs = nowInDay(options.nowMs, dayStartMs, dayEndMs);
  const hours = localHourStarts(dayStartMs, dayEndMs, timeZone);
  const placed = workstreams.flatMap((workstream) => {
    const entry = placeWorkstream(workstream, hours, dayStartMs, dayEndMs);
    return entry ? [entry] : [];
  });
  if (placed.length === 0) return EMPTY_LAYOUT;
  const busySegments = placed.flatMap((entry) => entry.whole.segments);
  const axis = layoutAxis(busySegments, hours, {
    dayStartMs,
    dayEndMs,
    nowMs,
    canvasHeight: options.canvasHeight,
  });
  const { toPx } = axis;

  const cards = placed.flatMap(({ stretches }) =>
    stretches.map((stretch, part): PlacedCard => {
      const cardTop = toPx(stretch.startMs);
      const realHeight = toPx(stretch.endMs) - cardTop;
      const cardHeight = Math.max(realHeight, minHeight);
      return {
        ...stretch,
        key: `${stretch.workstream.id}-${part}`,
        part,
        lane: 0,
        laneCount: 1,
        top: cardTop,
        height: cardHeight,
        realHeight,
        bars: barsFor(stretch.segments, cardTop, cardHeight, toPx),
        density: densityFor(cardHeight, textScale),
      };
    }),
  );
  assignLanes(cards, minHeight, gap);

  cards.sort(
    (a, b) => a.startMs - b.startMs || a.workstream.id - b.workstream.id || a.part - b.part,
  );
  const firstCard = new Map<number, number>();
  cards.forEach((card, index) => {
    if (!firstCard.has(card.workstream.id)) firstCard.set(card.workstream.id, index);
  });
  return {
    workstreams: placed
      .map((entry) => entry.whole)
      .sort(
        (a, b) => (firstCard.get(a.workstream.id) ?? 0) - (firstCard.get(b.workstream.id) ?? 0),
      ),
    cards: cards.map(({ realHeight: _realHeight, ...card }) => card),
    hours: axis.hours,
    height: Math.max(axis.height, ...cards.map((card) => card.top + card.height)),
    pxPerMin: axis.pxPerMin,
    toPx,
  };
}

/**
 * The grid of a day with no workstream yet: the same hour axis, with no
 * cards. The captured activity stands in for the work, so the window and
 * scale match what the grid shows once the hours are summarized; today
 * without any captured activity, the window ends at now.
 */
export function layoutEmptyTimeline(
  activity: Range[],
  options: {
    dayStartMs: number;
    dayEndMs: number;
    /** Today's current time; null on other days. */
    nowMs?: number | null;
    timeZone?: string;
    canvasHeight?: number;
  },
): WorkstreamTimelineLayout {
  const { dayStartMs, dayEndMs, timeZone } = options;
  const nowMs = nowInDay(options.nowMs, dayStartMs, dayEndMs);
  let work = activity.flatMap((range) => {
    const startMs = Math.max(range.startMs, dayStartMs);
    const endMs = Math.min(range.endMs, dayEndMs);
    return endMs > startMs ? [{ startMs, endMs }] : [];
  });
  if (work.length === 0) {
    // A past day with nothing captured has no hours to anchor a grid on.
    if (nowMs === null) return EMPTY_LAYOUT;
    work = [
      { startMs: Math.max(dayStartMs, nowMs - (MIN_WINDOW_MS - WINDOW_PAD_MS)), endMs: nowMs },
    ];
  }
  const hours = localHourStarts(dayStartMs, dayEndMs, timeZone);
  const axis = layoutAxis(work, hours, {
    dayStartMs,
    dayEndMs,
    nowMs,
    canvasHeight: options.canvasHeight,
  });
  return { workstreams: [], cards: [], ...axis };
}
