import { IconCheckmark2Small } from "central-icons/IconCheckmark2Small";
import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { createPortal } from "react-dom";

export type SelectPopoverPlacement = "align-selected" | "below" | "above";

/**
 * Native-macOS popup behavior, shared by every select-trigger surface: the
 * popover prefers sliding up so the currently selected option lines up with
 * the trigger's position, and falls back to a plain below/above dropdown
 * when that would leave the panel (or the viewport).
 */
export function selectPopoverPlacement(
  anchor: HTMLElement | null,
  optionCount: number,
  selectedIndex: number,
): SelectPopoverPlacement {
  if (!anchor) return "align-selected";
  const rect = anchor.getBoundingClientRect();

  const viewportPadding = 12;
  const rowHeight = 28;
  // Vertical padding (2 x 4px) plus the 1px borders.
  const popoverChrome = 10;
  const popoverHeight = optionCount * rowHeight + popoverChrome;
  // Keep in sync with selectPopoverStyle's align-selected offset.
  const selectedOffset = 3 + selectedIndex * rowHeight;
  const panel = anchor.closest(".main-panel");
  const panelRect = panel?.getBoundingClientRect();
  const topBound = Math.max(viewportPadding, (panelRect?.top ?? 0) + 12);
  const bottomBound = Math.min(
    window.innerHeight - viewportPadding,
    (panelRect?.bottom ?? window.innerHeight) - 12,
  );
  const alignedTop = rect.top - selectedOffset;
  const alignedBottom = alignedTop + popoverHeight;
  const belowBottom = rect.bottom + 4 + popoverHeight;
  const aboveTop = rect.top - 4 - popoverHeight;
  const spaceBelow = bottomBound - rect.bottom;
  const spaceAbove = rect.top - topBound;

  if (alignedTop >= topBound && alignedBottom <= bottomBound) {
    return "align-selected";
  }
  if (belowBottom <= bottomBound || spaceBelow >= spaceAbove) {
    return "below";
  }
  return aboveTop >= topBound ? "above" : "below";
}

export function selectPopoverStyle(
  placement: SelectPopoverPlacement,
  selectedIndex: number,
): CSSProperties {
  if (placement === "below") {
    return { top: "calc(100% + 4px)" };
  }
  if (placement === "above") {
    return { bottom: "calc(100% + 4px)" };
  }
  // 3px = the popover's border (1) + padding (4) minus the trigger/row height
  // difference (32 - 28) / 2, so the selected row overlays the trigger with no
  // vertical jump.
  return { top: -(3 + selectedIndex * 28) };
}

const POPOVER_VIEWPORT_INSET = 12;
const POPOVER_MIN_WIDTH = 260;

/**
 * Keeps a portaled select popover in view without disturbing its trigger
 * alignment unless the viewport has no room to its right.
 */
export function selectPopoverHorizontalStyle(
  rect: DOMRect,
  minimumWidth = POPOVER_MIN_WIDTH,
): CSSProperties {
  const viewportWidth = Math.max(0, window.innerWidth);
  const maxViewportWidth = Math.max(0, viewportWidth - POPOVER_VIEWPORT_INSET * 2);
  const width = Math.min(Math.max(rect.width, minimumWidth), maxViewportWidth);
  const left = Math.min(
    Math.max(rect.left, POPOVER_VIEWPORT_INSET),
    Math.max(POPOVER_VIEWPORT_INSET, viewportWidth - width - POPOVER_VIEWPORT_INSET),
  );

  return { left, width, minWidth: width, maxWidth: width };
}

/**
 * The same placement math as {@link selectPopoverStyle} but resolved to fixed
 * viewport coordinates, so the popover can render in a body portal (escaping any
 * ancestor `overflow: hidden`) while staying pinned to its trigger. `below`
 * anchors the popover top; `above` anchors its bottom to the viewport floor
 * (`window.innerHeight - triggerTop`); `align-selected` slides the selected row
 * over the trigger, matching the in-flow offset.
 */
function fixedVerticalStyle(
  placement: SelectPopoverPlacement,
  selectedIndex: number,
  rect: DOMRect,
): CSSProperties {
  if (placement === "below") {
    return { top: rect.bottom + 4 };
  }
  if (placement === "above") {
    return { bottom: window.innerHeight - rect.top + 4 };
  }
  return { top: rect.top - (3 + selectedIndex * 28) };
}

export type SelectOption = {
  value: string;
  label: string;
  /** Optional leading color chip (e.g. the accent picker). Omit for text
   * selects like Language/Microphone and no swatch renders. */
  color?: string;
  /** Optional trailing count, rendered as a small muted number badge rather than
   * inline in the label text (e.g. the skills category filter counts). */
  count?: number;
  /** BCP 47 tag when the label is written in another language than the
   * interface (the interface-language picker names each option natively). */
  lang?: string;
};

/**
 * The settings select (trigger + listbox popover) as a self-contained
 * control, for surfaces that don't hand-roll the open/placement state the
 * way AppSettings does. Same classes, so it is pixel-identical to the
 * Language and Microphone pickers.
 */
export function Select({
  value,
  options,
  placeholder,
  onChange,
  ariaLabel,
  className,
  popoverWidth = "content",
}: {
  value: string | null;
  options: SelectOption[];
  /** Trigger text while nothing is selected yet. */
  placeholder: string;
  onChange: (value: string) => void;
  ariaLabel: string;
  className?: string;
  /** Match the trigger for compact option sets such as the accent presets, or
   * an explicit minimum px width for triggers that hug their content (ghost
   * filter triggers), where neither the trigger nor the 260px default fits. */
  popoverWidth?: "content" | "trigger" | number;
}) {
  const [open, setOpen] = useState(false);
  const [placement, setPlacement] = useState<SelectPopoverPlacement>("align-selected");
  const wrapRef = useRef<HTMLDivElement>(null);
  const popoverRef = useRef<HTMLUListElement>(null);
  // The popover is rendered in a portal so it escapes any ancestor card's
  // `overflow: hidden` clipping; fixed coordinates are measured from the
  // trigger on open (and on scroll/resize while open).
  const [anchor, setAnchor] = useState<DOMRect | null>(null);

  const selectedIndex = options.findIndex((option) => option.value === value);
  const selected = selectedIndex === -1 ? undefined : options[selectedIndex];

  // Outside-press / Escape dismissal that accounts for the portaled popover:
  // a press inside either the trigger wrap or the popover keeps it open.
  useEffect(() => {
    if (!open) return;
    function onPointer(event: MouseEvent) {
      const target = event.target as Node;
      if (wrapRef.current?.contains(target) || popoverRef.current?.contains(target)) return;
      setOpen(false);
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  // Track the trigger rect while open so the fixed popover stays anchored
  // through scrolls and resizes.
  useLayoutEffect(() => {
    if (!open) return;
    const measure = () => {
      const el = wrapRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      setAnchor(rect);
    };
    measure();
    window.addEventListener("scroll", measure, true);
    window.addEventListener("resize", measure);
    return () => {
      window.removeEventListener("scroll", measure, true);
      window.removeEventListener("resize", measure);
    };
  }, [open]);

  function toggle() {
    if (!open) {
      // An unanswered select has no row to align with, so it opens as a
      // plain dropdown; Math.max keeps the offset math on row 0.
      setPlacement(
        selectedIndex === -1
          ? "below"
          : selectPopoverPlacement(wrapRef.current, options.length, selectedIndex),
      );
    }
    setOpen((current) => !current);
  }

  const horizontalStyle = anchor
    ? selectPopoverHorizontalStyle(
        anchor,
        typeof popoverWidth === "number"
          ? popoverWidth
          : popoverWidth === "trigger"
            ? Math.max(anchor.width, 1)
            : POPOVER_MIN_WIDTH,
      )
    : undefined;
  const fixedStyle: CSSProperties | undefined =
    horizontalStyle && horizontalStyle.width !== 0 && anchor
      ? {
          position: "fixed",
          ...horizontalStyle,
          ...fixedVerticalStyle(placement, Math.max(selectedIndex, 0), anchor),
        }
      : undefined;

  return (
    <div className={`select-control${className ? ` ${className}` : ""}`} ref={wrapRef}>
      <button
        type="button"
        className="select-trigger"
        data-placeholder={!selected}
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={toggle}
      >
        {selected?.color ? (
          <span className="select-swatch" style={{ background: selected.color }} aria-hidden />
        ) : null}
        <span lang={selected?.lang}>{selected?.label ?? placeholder}</span>
        {typeof selected?.count === "number" ? (
          <span className="status-pill select-count">{selected.count}</span>
        ) : null}
        <IconChevronDownSmall size={14} />
      </button>
      {open && fixedStyle
        ? createPortal(
            <ul
              ref={popoverRef}
              className="select-popover select-popover-portal"
              role="listbox"
              data-placement={placement}
              style={fixedStyle}
            >
              {renderOptions()}
            </ul>,
            document.body,
          )
        : null}
    </div>
  );

  function renderOptions() {
    return (
      <>
        {options.map((option) => {
          const isSelected = option.value === value;
          return (
            <li key={option.value} role="presentation">
              <button
                type="button"
                role="option"
                className={option.color ? "has-swatch" : undefined}
                aria-selected={isSelected}
                data-selected={isSelected}
                onClick={() => {
                  onChange(option.value);
                  setOpen(false);
                }}
              >
                {option.color ? (
                  <span
                    className="select-swatch"
                    style={{ background: option.color }}
                    aria-hidden
                  />
                ) : null}
                <span className="select-label" lang={option.lang}>
                  {option.label}
                </span>
                <span className="select-trailing">
                  <span className="select-check" aria-hidden>
                    {isSelected ? <IconCheckmark2Small size={14} /> : null}
                  </span>
                  {typeof option.count === "number" ? (
                    <span className="status-pill select-count">{option.count}</span>
                  ) : null}
                </span>
              </button>
            </li>
          );
        })}
      </>
    );
  }
}
