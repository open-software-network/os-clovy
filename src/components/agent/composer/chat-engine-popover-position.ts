import { type RefObject, useLayoutEffect } from "react";

export type ChatEnginePopoverPosition = {
  placement: "above" | "below";
  left: number;
  top?: number;
  bottom?: number;
  maxHeight: number;
};

/** Keeps the composer model menu pinned to its trigger and inside the viewport.
 * Above remains the preferred placement; below wins when the menu cannot fit
 * above and has more room there. Coordinates are relative to the composer form,
 * which is the popover's containing block. */
export function chatEnginePopoverPosition({
  triggerRect,
  anchorRect,
  popoverRect,
  viewportWidth,
  viewportHeight,
  topInset,
  gap,
  viewportMargin,
}: {
  triggerRect: DOMRect;
  anchorRect: DOMRect;
  popoverRect: DOMRect;
  viewportWidth: number;
  viewportHeight: number;
  topInset: number;
  gap: number;
  viewportMargin: number;
}): ChatEnginePopoverPosition {
  const topBound = topInset + viewportMargin;
  const bottomBound = viewportHeight - viewportMargin;
  const spaceAbove = Math.max(0, triggerRect.top - gap - topBound);
  const spaceBelow = Math.max(0, bottomBound - triggerRect.bottom - gap);
  const placement =
    popoverRect.height <= spaceAbove ||
    (popoverRect.height > spaceBelow && spaceAbove >= spaceBelow)
      ? "above"
      : "below";
  const maxLeft = Math.max(viewportMargin, viewportWidth - viewportMargin - popoverRect.width);
  const viewportLeft = Math.min(
    Math.max(triggerRect.right - popoverRect.width, viewportMargin),
    maxLeft,
  );
  const position: ChatEnginePopoverPosition = {
    placement,
    left: viewportLeft - anchorRect.left,
    maxHeight: Math.floor(placement === "above" ? spaceAbove : spaceBelow),
  };

  if (placement === "above") {
    position.bottom = anchorRect.bottom - triggerRect.top + gap;
  } else {
    position.top = triggerRect.bottom - anchorRect.top + gap;
  }
  return position;
}

export function useChatEnginePopoverPosition({
  open,
  triggerRef,
  popoverRef,
  anchorRef,
}: {
  open: boolean;
  triggerRef: RefObject<HTMLElement>;
  popoverRef: RefObject<HTMLElement>;
  anchorRef: RefObject<HTMLElement>;
}) {
  useLayoutEffect(() => {
    if (!open) return;
    const position = () => {
      const trigger = triggerRef.current;
      const popover = popoverRef.current;
      const anchor = anchorRef.current;
      if (!trigger || !popover || !anchor) return;

      const computed = getComputedStyle(popover);
      const cssPixels = (property: string, fallback: number) =>
        Number.parseFloat(computed.getPropertyValue(property)) || fallback;
      const next = chatEnginePopoverPosition({
        triggerRect: trigger.getBoundingClientRect(),
        anchorRect: anchor.getBoundingClientRect(),
        popoverRect: popover.getBoundingClientRect(),
        viewportWidth: window.innerWidth,
        viewportHeight: window.innerHeight,
        topInset: cssPixels("--titlebar-h", 28),
        gap: cssPixels("--sp-1", 4),
        viewportMargin: cssPixels("--sp-3", 12),
      });

      popover.dataset.placement = next.placement;
      popover.style.left = `${next.left}px`;
      popover.style.right = "auto";
      popover.style.maxHeight = `${next.maxHeight}px`;
      popover.style.top = next.top === undefined ? "auto" : `${next.top}px`;
      popover.style.bottom = next.bottom === undefined ? "auto" : `${next.bottom}px`;
    };

    position();
    const observer = new ResizeObserver(position);
    if (triggerRef.current) observer.observe(triggerRef.current);
    if (popoverRef.current) observer.observe(popoverRef.current);
    if (anchorRef.current) observer.observe(anchorRef.current);
    window.addEventListener("scroll", position, true);
    window.addEventListener("resize", position);
    return () => {
      observer.disconnect();
      window.removeEventListener("scroll", position, true);
      window.removeEventListener("resize", position);
    };
  }, [anchorRef, open, popoverRef, triggerRef]);
}
