import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent, MouseEvent, PointerEvent, ReactNode } from "react";
import { HoverTip } from "../ui/HoverTip";
import { primaryShortcutLabel } from "../../lib/platform";
import { useT } from "../../i18n";

export type TabItem = {
  id: string;
  title: string;
  icon: ReactNode;
};

type TabBarProps = {
  tabs: TabItem[];
  activeTabId: string;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onCloseOthers: (id: string) => void;
  onNew: () => void;
  // The visible tabs in their new left-to-right order after a drag-reorder.
  onReorder: (orderedVisibleIds: string[]) => void;
  layoutFrozen?: boolean;
  onDragRegionPointerDown?: (event: PointerEvent<HTMLDivElement>) => void;
};

type TabMenu = { tabId: string; x: number; y: number };

const MENU_WIDTH = 168;
// Layout budget used to decide how many tabs fit before the rest fold into the
// overflow popover. Kept in sync with the CSS (gap = --sp-2, the +/overflow
// buttons are ~24px). MIN_TAB is the narrowest a tab shrinks to — a centered
// icon — before it overflows into the popover; it must match the .tab
// min-width.
const GAP = 6;
const MIN_TAB = 40;
const BTN = 24;
// Width the active tab holds once the strip tightens below full size (tight
// and icon), so the current document stays prominent and readable. Keep in
// sync with --tab-active-compact-w in app.css.
const ACTIVE_COMPACT_TAB = 160;
// Movement (px) before a press on a tab becomes a drag instead of a click.
const DRAG_THRESHOLD = 4;

type Layout = { visible: TabItem[]; hidden: TabItem[] };

function computeLayout(
  tabs: TabItem[],
  activeTabId: string,
  available: number,
  // At compact sizes the active tab keeps a wide pill (label + close), so it
  // eats ACTIVE_COMPACT_TAB from the budget instead of MIN_TAB like the rest.
  activeWide = false,
): Layout {
  const activeExtra =
    activeWide && tabs.some((tab) => tab.id === activeTabId) ? ACTIVE_COMPACT_TAB - MIN_TAB : 0;
  // Reserve the "+" button. If every tab fits at MIN_TAB, show them all.
  const forAll = available - BTN - GAP - activeExtra;
  const capAll = Math.floor((forAll + GAP) / (MIN_TAB + GAP));
  if (tabs.length <= capAll || !Number.isFinite(available)) {
    return { visible: tabs, hidden: [] };
  }
  // Otherwise reserve the overflow button too and fold the rest away.
  const forSome = available - BTN - GAP - BTN - GAP - activeExtra;
  const count = Math.max(1, Math.floor((forSome + GAP) / (MIN_TAB + GAP)));
  const visible = tabs.slice(0, count);
  const hidden = tabs.slice(count);
  // The focused tab must stay on the strip: swap it into the last visible slot.
  if (!visible.some((t) => t.id === activeTabId)) {
    const active = tabs.find((t) => t.id === activeTabId);
    if (active && visible.length > 0) {
      const displaced = visible[visible.length - 1]!;
      visible[visible.length - 1] = active;
      return {
        visible,
        hidden: [displaced, ...hidden.filter((t) => t.id !== activeTabId)],
      };
    }
  }
  return { visible, hidden };
}

// The width each visible tab resolves to under flex (equal share of the space
// left after the +/overflow buttons and gaps), clamped like the CSS.
function effectiveTabWidth(count: number, hasOverflow: boolean, available: number): number {
  if (!Number.isFinite(available) || count <= 0) return Number.POSITIVE_INFINITY;
  const buttons = BTN + (hasOverflow ? BTN : 0);
  const items = count + (hasOverflow ? 1 : 0) + 1;
  const gaps = Math.max(0, items - 1) * GAP;
  const forTabs = available - buttons - gaps;
  const raw = forTabs / count;
  const maxW = Math.min(240, available * 0.25);
  return Math.max(MIN_TAB, Math.min(maxW, raw));
}

// Same, but for the inactive tabs once the active one holds its wide compact
// pill: they split what the pill leaves behind. Decides whether they can still
// carry labels ("tight") or fall back to bare icons.
function inactiveTabWidth(count: number, hasOverflow: boolean, available: number): number {
  // An active-only strip has no inactive tabs to size — read it as the icon
  // regime (just the wide pill).
  if (count <= 1) return 0;
  if (!Number.isFinite(available)) return Number.POSITIVE_INFINITY;
  const buttons = BTN + (hasOverflow ? BTN : 0);
  const items = count + (hasOverflow ? 1 : 0) + 1;
  const gaps = Math.max(0, items - 1) * GAP;
  const forTabs = available - buttons - gaps - ACTIVE_COMPACT_TAB;
  const raw = forTabs / (count - 1);
  const maxW = Math.min(240, available * 0.25);
  return Math.max(MIN_TAB, Math.min(maxW, raw));
}

// A drag in progress: created on pointerdown, armed once movement crosses
// DRAG_THRESHOLD, and settled (a short slide into the final slot) before the
// reorder commits.
type DragState = {
  pointerId: number;
  id: string;
  fromIndex: number;
  toIndex: number;
  startX: number;
  started: boolean;
  settling: boolean;
  visibleIds: string[];
  slots: { el: HTMLElement; left: number; width: number }[];
  // Last transform offset applied per slot, to skip redundant style writes.
  offsets: number[];
};

export function TabBar({
  tabs,
  activeTabId,
  onActivate,
  onClose,
  onCloseOthers,
  onNew,
  onReorder,
  layoutFrozen = false,
  onDragRegionPointerDown,
}: TabBarProps) {
  const stripRef = useRef<HTMLDivElement>(null);
  const layoutFrozenRef = useRef(layoutFrozen);
  const pendingAvailableRef = useRef<number | null>(null);
  const [available, setAvailable] = useState(Number.POSITIVE_INFINITY);
  const [menu, setMenu] = useState<TabMenu | null>(null);
  const [overflowOpen, setOverflowOpen] = useState(false);
  const dragRef = useRef<DragState | null>(null);
  const settleTimerRef = useRef<number | null>(null);
  // Set once a reorder commits: the tabs render in their new order on the next
  // pass, so the drag transforms that mimicked that order must be cleared
  // before paint (see the layout effect below).
  const pendingTransformClearRef = useRef(false);
  const [dragSourceId, setDragSourceId] = useState<string | null>(null);
  const newTabShortcut = primaryShortcutLabel("T");
  const t = useT();

  function stripTabEls(): HTMLElement[] {
    return Array.from(stripRef.current?.querySelectorAll<HTMLElement>(".tab") ?? []);
  }

  function clearDragTransforms() {
    for (const el of stripTabEls()) {
      el.style.transform = "";
      el.style.transition = "";
    }
  }

  // The tabs themselves are user-select: none, but the native selection drag
  // keeps running page-wide once the pointer leaves the strip (pointer capture
  // retargets pointer events, not WebKit's selection machinery) — so text
  // elsewhere gets highlighted mid-drag unless selection is locked globally.
  function lockSelection() {
    document.body.style.userSelect = "none";
    document.body.style.webkitUserSelect = "none";
    window.getSelection()?.removeAllRanges();
  }

  function unlockSelection() {
    document.body.style.userSelect = "";
    document.body.style.webkitUserSelect = "";
  }

  function abortDrag() {
    if (!dragRef.current) return;
    if (settleTimerRef.current !== null) {
      window.clearTimeout(settleTimerRef.current);
      settleTimerRef.current = null;
    }
    dragRef.current = null;
    clearDragTransforms();
    unlockSelection();
    setDragSourceId(null);
  }

  function setMeasuredAvailable(width: number) {
    // A live drag can't survive its slot geometry changing under it — but a
    // settling drag is already dropped and commits momentarily, so let it.
    const drag = dragRef.current;
    if (drag?.started && !drag.settling) abortDrag();
    if (layoutFrozenRef.current) {
      pendingAvailableRef.current = width;
      return;
    }
    setAvailable(width);
  }

  useLayoutEffect(() => {
    layoutFrozenRef.current = layoutFrozen;
    if (layoutFrozen) return;
    const width = pendingAvailableRef.current ?? stripRef.current?.clientWidth;
    pendingAvailableRef.current = null;
    if (typeof width === "number") setAvailable(width);
  }, [layoutFrozen]);

  // Track the strip's content width so we know how many tabs fit.
  useLayoutEffect(() => {
    const el = stripRef.current;
    if (!el) return;
    setMeasuredAvailable(el.clientWidth);
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      const width = entries[0]?.contentRect.width;
      if (typeof width === "number") setMeasuredAvailable(width);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // How wide each visible tab would render if they all shared evenly, so the
  // strip can shed labels as it tightens — deterministic (vs. relying on CSS
  // container queries to fire) since we already know the width. Below full
  // size the layout is recomputed with the active tab held wide
  // (ACTIVE_COMPACT_TAB) so the current document stays prominent — that
  // reservation can fold a few more tabs into overflow. Whether the inactive
  // tabs then keep their labels ("tight") or fall back to bare icons ("icon")
  // follows from the width the pill leaves them.
  const uniform = computeLayout(tabs, activeTabId, available);
  const uniformWidth = effectiveTabWidth(
    uniform.visible.length,
    uniform.hidden.length > 0,
    available,
  );
  const activeWide = tabs.length > 1 && uniformWidth < 120;
  const { visible, hidden } = activeWide
    ? computeLayout(tabs, activeTabId, available, true)
    : uniform;
  const size = !activeWide
    ? "full"
    : inactiveTabWidth(visible.length, hidden.length > 0, available) < 64
      ? "icon"
      : "tight";

  // After a reorder commits, React re-renders the tabs in the order the drag
  // transforms were faking — drop the transforms in the same frame (before
  // paint) so nothing jumps.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `tabs` is the trigger, not a read.
  useLayoutEffect(() => {
    if (!pendingTransformClearRef.current) return;
    pendingTransformClearRef.current = false;
    clearDragTransforms();
  }, [tabs]);

  useEffect(() => {
    return () => {
      if (settleTimerRef.current !== null) window.clearTimeout(settleTimerRef.current);
      // Unmounting mid-drag must not leave the page selection-locked.
      if (dragRef.current) unlockSelection();
    };
  }, []);

  // The overflow popover is meaningless once everything fits again.
  useEffect(() => {
    if (hidden.length === 0) setOverflowOpen(false);
  }, [hidden.length]);

  // Dismiss the right-click menu / overflow popover on outside click or Escape.
  useEffect(() => {
    if (!menu && !overflowOpen) return;
    const close = () => {
      setMenu(null);
      setOverflowOpen(false);
    };
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("click", close);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu, overflowOpen]);

  function handleTabKeyDown(event: KeyboardEvent<HTMLDivElement>, id: string) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onActivate(id);
    }
  }

  function handleAuxClick(event: MouseEvent<HTMLDivElement>, id: string) {
    // Middle-click closes, matching every browser.
    if (event.button === 1) {
      event.preventDefault();
      onClose(id);
    }
  }

  function handleContextMenu(event: MouseEvent<HTMLDivElement>, id: string) {
    // preventDefault both opens our menu and signals the native context-menu
    // guard to stand down.
    event.preventDefault();
    setOverflowOpen(false);
    setMenu({
      tabId: id,
      x: Math.min(event.clientX, window.innerWidth - MENU_WIDTH),
      y: event.clientY,
    });
  }

  function handleDragRegionPointerDown(event: PointerEvent<HTMLDivElement>) {
    if (event.target !== event.currentTarget) return;
    onDragRegionPointerDown?.(event);
  }

  function handleTabPointerDown(event: PointerEvent<HTMLDivElement>, id: string, index: number) {
    // Only a primary-button press on a multi-tab strip can become a drag.
    if (event.button !== 0 || dragRef.current || visible.length <= 1) return;
    dragRef.current = {
      pointerId: event.pointerId,
      id,
      fromIndex: index,
      toIndex: index,
      startX: event.clientX,
      started: false,
      settling: false,
      visibleIds: visible.map((tab) => tab.id),
      slots: [],
      offsets: [],
    };
    // Retarget the pointer stream to this tab for the whole gesture (absent in
    // jsdom, hence the optional call).
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function handleTabPointerMove(event: PointerEvent<HTMLDivElement>) {
    const drag = dragRef.current;
    if (!drag || drag.settling || event.pointerId !== drag.pointerId) return;
    const dx = event.clientX - drag.startX;
    if (!drag.started) {
      if (Math.abs(dx) < DRAG_THRESHOLD) return;
      // Snapshot the slot geometry once, at drag start — every shift below is
      // computed against it, so mid-drag reflows can't skew the math.
      drag.slots = stripTabEls().map((el) => {
        const rect = el.getBoundingClientRect();
        return { el, left: rect.left, width: rect.width };
      });
      if (drag.slots.length !== drag.visibleIds.length) {
        dragRef.current = null;
        return;
      }
      drag.offsets = drag.slots.map(() => 0);
      drag.started = true;
      lockSelection();
      setDragSourceId(drag.id);
    }
    const { slots, fromIndex } = drag;
    const mine = slots[fromIndex]!;
    const first = slots[0]!;
    const last = slots[slots.length - 1]!;
    // The dragged tab tracks the pointer, clamped to the strip's tab run.
    const clamped = Math.max(
      first.left - mine.left,
      Math.min(last.left + last.width - mine.width - mine.left, dx),
    );
    // The destination is the slot whose landing position (where the dragged
    // tab's left edge would settle if dropped there) is nearest its current
    // left edge. Unlike center-crossing this stays honest with mixed widths —
    // a wide active pill swaps with a 40px icon tab after ~23px of travel,
    // not after overshooting the icon's faraway center.
    const current = mine.left + clamped;
    let toIndex = fromIndex;
    let bestDistance = Number.POSITIVE_INFINITY;
    for (let i = 0; i < slots.length; i += 1) {
      const slot = slots[i]!;
      const landing =
        i === fromIndex
          ? mine.left
          : i > fromIndex
            ? slot.left + slot.width - mine.width
            : slot.left;
      const distance = Math.abs(current - landing);
      if (distance < bestDistance) {
        bestDistance = distance;
        toIndex = i;
      }
    }
    drag.toIndex = toIndex;
    // Displaced neighbors slide over by the dragged tab's footprint (their
    // transition comes from the data-dragging CSS); the dragged tab itself
    // moves transition-free under the pointer. Only write transforms that
    // changed — same-value writes still dirty style on every pointermove.
    const shift = mine.width + GAP;
    slots.forEach((slot, i) => {
      if (i === fromIndex) return;
      let offset = 0;
      if (fromIndex < toIndex && i > fromIndex && i <= toIndex) offset = -shift;
      else if (toIndex < fromIndex && i >= toIndex && i < fromIndex) offset = shift;
      if (drag.offsets[i] !== offset) {
        drag.offsets[i] = offset;
        slot.el.style.transform = offset ? `translateX(${offset}px)` : "";
      }
    });
    mine.el.style.transform = `translateX(${clamped}px)`;
  }

  function handleTabPointerUp(event: PointerEvent<HTMLDivElement>) {
    const drag = dragRef.current;
    if (!drag || drag.settling || event.pointerId !== drag.pointerId) return;
    if (!drag.started) {
      // A plain press: hand back to the click handler for activation.
      dragRef.current = null;
      return;
    }
    // Slide the dragged tab the rest of the way into its slot, then commit.
    drag.settling = true;
    const { slots, fromIndex, toIndex } = drag;
    const mine = slots[fromIndex]!;
    const target = slots[toIndex]!;
    const finalLeft = toIndex > fromIndex ? target.left + target.width - mine.width : target.left;
    mine.el.style.transition = "transform 160ms var(--ease-out)";
    mine.el.style.transform = `translateX(${finalLeft - mine.left}px)`;
    settleTimerRef.current = window.setTimeout(() => {
      settleTimerRef.current = null;
      commitDrag();
    }, 170);
  }

  function commitDrag() {
    const drag = dragRef.current;
    if (!drag?.started) return;
    dragRef.current = null;
    if (drag.fromIndex === drag.toIndex) {
      // Nothing moved: no re-render is coming, so clean up here.
      clearDragTransforms();
    } else {
      pendingTransformClearRef.current = true;
      const ids = [...drag.visibleIds];
      const [moved] = ids.splice(drag.fromIndex, 1);
      ids.splice(drag.toIndex, 0, moved!);
      onReorder(ids);
    }
    // Grabbing a tab focuses it, like a browser — but only on drop, so the
    // slot widths can't shift mid-drag (the active tab is wider at icon size).
    onActivate(drag.id);
    unlockSelection();
    setDragSourceId(null);
  }

  function handleTabPointerCancel(event: PointerEvent<HTMLDivElement>) {
    const drag = dragRef.current;
    if (!drag || drag.settling || event.pointerId !== drag.pointerId) return;
    abortDrag();
  }

  function renderTab(tab: TabItem, index: number) {
    const active = tab.id === activeTabId;
    return (
      <div
        key={tab.id}
        className="tab"
        role="tab"
        tabIndex={0}
        aria-selected={active}
        data-active={active || undefined}
        data-has-icon={tab.icon ? true : undefined}
        data-drag-source={tab.id === dragSourceId || undefined}
        title={tab.title}
        onClick={() => {
          // The click that follows a drag's pointerup must not re-activate.
          if (dragRef.current) return;
          onActivate(tab.id);
        }}
        onAuxClick={(event) => handleAuxClick(event, tab.id)}
        onContextMenu={(event) => handleContextMenu(event, tab.id)}
        onKeyDown={(event) => handleTabKeyDown(event, tab.id)}
        onPointerDown={(event) => handleTabPointerDown(event, tab.id, index)}
        onPointerMove={handleTabPointerMove}
        onPointerUp={handleTabPointerUp}
        onPointerCancel={handleTabPointerCancel}
      >
        {tab.icon ? (
          <span className="tab-icon" aria-hidden>
            {tab.icon}
          </span>
        ) : null}
        <span className="tab-label">{tab.title}</span>
        <button
          type="button"
          className="tab-close"
          tabIndex={-1}
          aria-hidden="true"
          aria-label={t("shell.tabs.closeTab", { title: tab.title })}
          onClick={(event) => {
            event.stopPropagation();
            onClose(tab.id);
          }}
          onPointerDown={(event) => event.stopPropagation()}
        >
          <IconCrossSmall size={12} />
        </button>
      </div>
    );
  }

  return (
    <div
      className="tab-bar"
      role="tablist"
      aria-label={t("shell.tabs.openTabs")}
      data-tauri-drag-region
      onPointerDown={handleDragRegionPointerDown}
    >
      <div
        className="tab-strip"
        ref={stripRef}
        data-size={size}
        data-dragging={dragSourceId ? "" : undefined}
        data-tauri-drag-region
        onPointerDown={handleDragRegionPointerDown}
      >
        {visible.map(renderTab)}
        {hidden.length > 0 ? (
          <button
            type="button"
            className="tab-overflow"
            // The popover is a full switcher listing every open tab (the badge
            // counts how many are currently off-strip), so the label says so.
            aria-label={t("shell.tabs.showAll", { count: tabs.length })}
            aria-expanded={overflowOpen}
            onClick={(event) => {
              event.stopPropagation();
              setMenu(null);
              setOverflowOpen((open) => !open);
            }}
          >
            <span className="tab-overflow-count">{hidden.length}</span>
            <IconChevronDownSmall size={14} />
          </button>
        ) : null}
        <HoverTip
          className="tab-new-anchor"
          compact
          // "Ctrl T" (Windows) needs more room than the tight "⌘T".
          width={newTabShortcut.length > 3 ? 132 : 100}
          delay={550}
          tip={
            <span className="tab-tip">
              {t("shell.tabs.newTab")}
              <span className="tab-tip-kbd">{newTabShortcut}</span>
            </span>
          }
        >
          <button
            type="button"
            className="tab-new"
            aria-label={t("shell.tabs.newTab")}
            onClick={onNew}
          >
            <IconPlusMedium size={14} />
          </button>
        </HoverTip>
      </div>

      {overflowOpen ? (
        <div
          className="tab-overflow-popover"
          role="menu"
          onClick={(event) => event.stopPropagation()}
        >
          {tabs.map((tab) => {
            const active = tab.id === activeTabId;
            return (
              <div
                key={tab.id}
                className="tab-overflow-item"
                role="menuitem"
                tabIndex={0}
                data-active={active || undefined}
                onClick={() => {
                  onActivate(tab.id);
                  setOverflowOpen(false);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault();
                    onActivate(tab.id);
                    setOverflowOpen(false);
                  }
                }}
              >
                {tab.icon ? (
                  <span className="tab-overflow-icon" aria-hidden>
                    {tab.icon}
                  </span>
                ) : null}
                <span className="tab-overflow-title">{tab.title}</span>
                <button
                  type="button"
                  className="tab-overflow-close"
                  tabIndex={-1}
                  aria-hidden="true"
                  aria-label={t("shell.tabs.closeTab", { title: tab.title })}
                  onClick={(event) => {
                    event.stopPropagation();
                    onClose(tab.id);
                  }}
                >
                  <IconCrossSmall size={12} />
                </button>
              </div>
            );
          })}
        </div>
      ) : null}

      {menu ? (
        <div
          className="context-menu"
          style={{ left: menu.x, top: menu.y }}
          role="menu"
          onClick={(event) => event.stopPropagation()}
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              onClose(menu.tabId);
              setMenu(null);
            }}
          >
            {t("shell.tabs.closeTabMenu")}
          </button>
          {tabs.length > 1 ? (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                onCloseOthers(menu.tabId);
                setMenu(null);
              }}
            >
              {t("shell.tabs.closeOthers")}
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
