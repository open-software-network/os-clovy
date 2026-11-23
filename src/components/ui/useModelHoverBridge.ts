import { type RefObject, useEffect, useRef, useState } from "react";

import type { ProviderModelDto } from "../../lib/tauri";
import {
  createHoverBridgeTracker,
  type HoverBridgePoint,
  type HoverBridgeRect,
  pointInRect,
  rectFromElement,
} from "./hoverBridge";

// The picker flyout state, shared verbatim by the composer and settings pickers
// (both declare a structurally identical alias). A row's detail card is open
// when `kind === "model"`; the searchable catalog when `kind === "all"`.
// The bridge only traverses model detail cards.
export type ModelHoverFlyout = { kind: "model"; id: string } | { kind: "all" } | null;

// Reports whether a safe-polygon traversal is currently anchored, so the rows
// can suppress their own hover-intent while the pointer is bridging to a card.
export type HoverBridgeHandle = { isActive: () => boolean };

/**
 * Suggested-row detail flyout bridge. A single window listener drives the whole
 * safe polygon: leaving the active row anchors a fresh wedge toward the card;
 * while the wedge holds, the card stays put and other rows' hover is
 * suppressed (via `setBridging`). When the wedge is dropped (pointer left it or
 * stalled), hover is handed to the row now under the pointer, which re-opens
 * its own card. Extracted so the composer and settings pickers share one copy.
 */
export function useModelDetailHoverBridge({
  flyout,
  popoverRef,
  cardRef,
  cancelHoverIntent,
  setBridging,
  onFlyoutChange,
}: {
  flyout: ModelHoverFlyout;
  popoverRef: RefObject<HTMLElement | null>;
  cardRef: RefObject<HTMLElement | null>;
  cancelHoverIntent: () => void;
  setBridging: (on: boolean) => void;
  onFlyoutChange: (flyout: ModelHoverFlyout) => void;
}): HoverBridgeHandle {
  // `onExpire` (pointer stalled inside the wedge) routes through a ref so it
  // always runs the latest hand-off closure.
  const handoffRef = useRef<(point: HoverBridgePoint) => void>(() => {});
  const [tracker] = useState(() =>
    createHoverBridgeTracker({ onExpire: (point) => handoffRef.current(point) }),
  );
  const anchorRef = useRef(false);

  useEffect(() => {
    if (flyout?.kind !== "model") return;
    // The flyout just opened because the pointer (or focus) is on a row.
    anchorRef.current = true;

    const activeRow = () =>
      popoverRef.current?.querySelector<HTMLElement>(
        '.agent-composer-model-row[data-active="true"]',
      );

    function handoff(point: HoverBridgePoint) {
      const target = document
        .elementFromPoint(point.x, point.y)
        ?.closest<HTMLElement>(".agent-composer-model-row");
      if (target && popoverRef.current?.contains(target)) {
        // Re-targeting another row: keep the bridging marker suppressing the new
        // row's raw :hover through the hover-intent delay, and lift it only as
        // the new card opens (when data-active transfers). Otherwise the still-
        // open card's row and the freshly hovered row both read as highlighted
        // during the delay.
        if (target.classList.contains("agent-composer-model-all")) {
          cancelHoverIntent();
          onFlyoutChange({ kind: "all" });
          setBridging(false);
          return;
        }
        const id = target.getAttribute("data-model-id");
        if (id) {
          cancelHoverIntent();
          onFlyoutChange({ kind: "model", id });
          setBridging(false);
          return;
        }
      }
      // No re-target (pointer left the list): lift the suppression immediately.
      setBridging(false);
      onFlyoutChange(null);
    }
    handoffRef.current = handoff;

    function handlePointerMove(event: PointerEvent) {
      const point = { x: event.clientX, y: event.clientY };
      const card = cardRef.current;
      const row = activeRow();
      if (!card || !row) return;
      if (pointInRect(point, rectFromElement(card)) || pointInRect(point, rectFromElement(row))) {
        anchorRef.current = true;
        cancelHoverIntent();
        tracker.stop();
        setBridging(false);
        return;
      }
      if (tracker.isActive()) {
        if (tracker.update(point)) setBridging(true);
        else handoff(point);
        return;
      }
      if (anchorRef.current) {
        anchorRef.current = false;
        const rowRect = rectFromElement(row);
        const cardRect = rectFromElement(card);
        const side = cardRect.left >= rowRect.right ? "right" : "left";
        tracker.begin(point, rowRect, cardRect, side);
        setBridging(true);
      }
    }
    window.addEventListener("pointermove", handlePointerMove, true);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove, true);
      tracker.stop();
      setBridging(false);
    };
  }, [flyout, tracker, onFlyoutChange, popoverRef, cardRef, cancelHoverIntent, setBridging]);

  return { isActive: tracker.isActive };
}

// The catalog hovercard's anchoring: the hovered row's rect and the card's
// preferred side, computed by the caller's `showCatalogHover`.
export type CatalogHoverAnchor = {
  model: ProviderModelDto;
  rowRect: HoverBridgeRect;
  side: "left" | "right";
};

/**
 * Catalog hovercard bridge — the same safe-polygon machine, anchored on the
 * hovered catalog row and its card. Works for both the "All models" panel and
 * the direct-catalog popover (the list lives under `listRef` either way).
 */
export function useCatalogHoverBridge({
  catalogHover,
  cardRef,
  listRef,
  resolveOption,
  showCatalogHover,
  cancelHoverIntent,
  cancelCatalogClose,
  scheduleCatalogClose,
  setBridging,
}: {
  catalogHover: CatalogHoverAnchor | null;
  cardRef: RefObject<HTMLElement | null>;
  listRef: RefObject<HTMLElement | null>;
  resolveOption: (id: string) => ProviderModelDto | undefined;
  showCatalogHover: (option: ProviderModelDto, row: HTMLElement) => void;
  cancelHoverIntent: () => void;
  cancelCatalogClose: () => void;
  scheduleCatalogClose: () => void;
  setBridging: (on: boolean) => void;
}): HoverBridgeHandle {
  const handoffRef = useRef<(point: HoverBridgePoint) => void>(() => {});
  const [tracker] = useState(() =>
    createHoverBridgeTracker({ onExpire: (point) => handoffRef.current(point) }),
  );
  const anchorRef = useRef(false);

  useEffect(() => {
    if (!catalogHover) return;
    anchorRef.current = true;
    const { rowRect, side } = catalogHover;

    function handoff(point: HoverBridgePoint) {
      const target = document
        .elementFromPoint(point.x, point.y)
        ?.closest<HTMLElement>(".agent-composer-model-row");
      if (target && listRef.current?.contains(target)) {
        const id = target.getAttribute("data-model-id");
        const option = id ? resolveOption(id) : undefined;
        if (option) {
          cancelHoverIntent();
          showCatalogHover(option, target);
          setBridging(false);
          return;
        }
      }
      // No re-target: lift the suppression immediately.
      setBridging(false);
      scheduleCatalogClose();
    }
    handoffRef.current = handoff;

    function handlePointerMove(event: PointerEvent) {
      const point = { x: event.clientX, y: event.clientY };
      const card = cardRef.current;
      if (!card) return;
      if (pointInRect(point, rectFromElement(card)) || pointInRect(point, rowRect)) {
        anchorRef.current = true;
        cancelHoverIntent();
        cancelCatalogClose();
        tracker.stop();
        setBridging(false);
        return;
      }
      if (tracker.isActive()) {
        if (tracker.update(point)) {
          setBridging(true);
          cancelCatalogClose();
        } else {
          handoff(point);
        }
        return;
      }
      if (anchorRef.current) {
        anchorRef.current = false;
        tracker.begin(point, rowRect, rectFromElement(card), side);
        setBridging(true);
        cancelCatalogClose();
      }
    }
    window.addEventListener("pointermove", handlePointerMove, true);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove, true);
      tracker.stop();
      setBridging(false);
    };
  }, [
    catalogHover,
    tracker,
    cardRef,
    listRef,
    resolveOption,
    showCatalogHover,
    cancelHoverIntent,
    cancelCatalogClose,
    scheduleCatalogClose,
    setBridging,
  ]);

  return { isActive: tracker.isActive };
}
