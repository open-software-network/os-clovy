import { IconChevronRightSmall } from "central-icons/IconChevronRightSmall";
import { IconCheckmark2Small } from "central-icons-filled/IconCheckmark2Small";
import {
  Fragment,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import type { RefObject } from "react";
import { createPortal } from "react-dom";
import { type MessageKey, useT } from "../../i18n";
import { modelAvailableForMode, modelIsPrivate, modelPrivacyBadge } from "../../lib/model-privacy";
import { modelMatchesQuery } from "../../lib/model-search";
import { suggestedModelsForMode } from "../../lib/suggested-models";
import type { ProviderModelDto, ProviderModelMode } from "../../lib/tauri";
import { useScrollFade } from "../../lib/use-scroll-fade";
import { rectFromElement, type HoverBridgeRect } from "../ui/hoverBridge";
import { useCatalogHoverBridge, useModelDetailHoverBridge } from "../ui/useModelHoverBridge";
import { ModelPrivacyChip, ModelRowPrivacyBadge } from "../ui/ModelPrivacyChip";
import { Switch } from "../ui/Switch";
import { modelSpecEntries } from "./ModelPickerDialog";
import { ProviderLogo } from "./ProviderLogo";

export type ModelPickerFlyout = { kind: "model"; id: string } | { kind: "all" } | null;

const MODE_LABEL_KEYS = {
  generation: {
    choose: "settingsPanels.picker.chooseText",
    suggested: "settingsPanels.picker.suggestedText",
    all: "settingsPanels.picker.allText",
  },
  transcription: {
    choose: "settingsPanels.picker.chooseTranscription",
    suggested: "settingsPanels.picker.suggestedTranscription",
    all: "settingsPanels.picker.allTranscription",
  },
  image: {
    choose: "settingsPanels.picker.chooseImage",
    suggested: "settingsPanels.picker.suggestedImage",
    all: "settingsPanels.picker.allImage",
  },
  video: {
    choose: "settingsPanels.picker.chooseVideo",
    suggested: "settingsPanels.picker.suggestedVideo",
    all: "settingsPanels.picker.allVideo",
  },
} as const satisfies Record<
  ProviderModelMode,
  { choose: MessageKey; suggested: MessageKey; all: MessageKey }
>;

// Row hovers should feel quick while moving through models, but still keep a
// tiny intent delay so a pointer sweep does not flash every card open.
const MODEL_HOVER_OPEN_INTENT_MS = 45;
const MODEL_HOVER_CLOSE_INTENT_MS = 150;
const MODEL_HOVERCARD_W = 248;
const MODEL_HOVERCARD_GAP = 4;
const MODEL_HOVERCARD_VIEWPORT_MARGIN = 12;

export function ModelPickerPopover({
  mode,
  flyout,
  model,
  options,
  search,
  popoverRef,
  searchRef,
  className,
  title: titleProp,
  ariaLabel: ariaLabelProp,
  suggestedListLabel: suggestedListLabelProp,
  allModelsLabel: allModelsLabelProp,
  suggestedModelIds,
  rootSearchRef,
  rootSearch,
  onRootSearchChange,
  onFlyoutChange,
  onSearchChange,
  onSelect,
}: {
  mode: ProviderModelMode;
  flyout: ModelPickerFlyout;
  model?: ProviderModelDto;
  options: ProviderModelDto[];
  search: string;
  popoverRef: RefObject<HTMLDivElement>;
  searchRef: RefObject<HTMLInputElement>;
  className?: string;
  title?: string;
  ariaLabel?: string;
  suggestedListLabel?: string;
  allModelsLabel?: string;
  /** Optional suggestion order for the curated media lists. */
  suggestedModelIds?: readonly string[];
  /** Enables the root-layer search (the /model + composer trigger surface):
   * a field above the pinned controls whose query searches across BOTH
   * layers, suggested picks and the full catalog, as one flat result list.
   * The All models flyout keeps its own independent field and query: L2's
   * box filters only the catalog list, and typing there never flips the
   * root layer into results mode. */
  rootSearchRef?: RefObject<HTMLInputElement>;
  rootSearch?: string;
  onRootSearchChange?: (value: string) => void;
  onFlyoutChange: (flyout: ModelPickerFlyout) => void;
  onSearchChange: (value: string) => void;
  onSelect: (modelId: string) => void;
}) {
  const t = useT();
  const modeLabels = MODE_LABEL_KEYS[mode];
  const title = titleProp ?? t("settingsPanels.picker.suggested");
  const ariaLabel = ariaLabelProp ?? t(modeLabels.choose);
  const suggestedListLabel = suggestedListLabelProp ?? t(modeLabels.suggested);
  const allModelsLabel = allModelsLabelProp ?? t(modeLabels.all);
  const flyoutRef = useRef<HTMLDivElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const hovercardRef = useRef<HTMLDivElement | null>(null);
  // "Private" catalog filter. Local to the popover on purpose: it resets when
  // the picker closes, so a stale filter can never silently hide models on the
  // next open.
  const [privateOnly, setPrivateOnly] = useState(false);
  // The suggested-row detail card is portaled to document.body (so a scroll
  // container or panel can't clip or cover it) and positioned in viewport
  // coordinates beside the popover — the same mechanism as the catalog
  // hovercard below.
  const [detailPos, setDetailPos] = useState<{
    top: number;
    x: number;
    side: "left" | "right";
  } | null>(null);
  const [catalogHover, setCatalogHover] = useState<{
    model: ProviderModelDto;
    rowRect: HoverBridgeRect;
    top: number;
    x: number;
    side: "left" | "right";
  } | null>(null);
  const hoverTimerRef = useRef<number | null>(null);
  const cancelHoverIntent = useCallback(() => {
    if (hoverTimerRef.current !== null) {
      window.clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  }, []);
  const hoverIntent = useCallback(
    (action: () => void) => {
      cancelHoverIntent();
      hoverTimerRef.current = window.setTimeout(action, MODEL_HOVER_OPEN_INTENT_MS);
    },
    [cancelHoverIntent],
  );
  useEffect(() => cancelHoverIntent, [cancelHoverIntent]);

  const closeTimerRef = useRef<number | null>(null);
  const cancelCatalogClose = useCallback(() => {
    if (closeTimerRef.current !== null) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
  }, []);
  const scheduleCatalogClose = useCallback(() => {
    cancelCatalogClose();
    closeTimerRef.current = window.setTimeout(
      () => setCatalogHover(null),
      MODEL_HOVER_CLOSE_INTENT_MS,
    );
  }, [cancelCatalogClose]);
  useEffect(() => cancelCatalogClose, [cancelCatalogClose]);

  // While a safe-polygon traversal is in flight, a `data-hover-bridging` marker
  // on the popover suppresses the CSS `:hover` highlight on every non-active row
  // so only one row ever reads as active (see app.css). Toggled imperatively to
  // avoid a re-render on every pointermove.
  const setBridging = useCallback(
    (on: boolean) => {
      const el = popoverRef.current;
      if (!el) return;
      if (on) el.setAttribute("data-hover-bridging", "true");
      else el.removeAttribute("data-hover-bridging");
    },
    [popoverRef],
  );

  const portalTarget = typeof document === "undefined" ? null : document.body;

  const fade = useScrollFade(listRef);

  // The all-models panel stays anchored to the menu's bottom edge and grows
  // upward; cap its height to the room above (clearing the titlebar strip that
  // would otherwise cover the search field) and a fixed ceiling. (The
  // suggested-model detail card is positioned separately, below, since it's
  // portaled to the body.)
  useLayoutEffect(() => {
    if (flyout?.kind !== "all") return;
    const el = flyoutRef.current;
    if (!el) return;
    el.dataset.side = "left";
    el.style.top = "";
    el.style.bottom = "";
    const titlebar = parseFloat(getComputedStyle(el).getPropertyValue("--titlebar-h")) || 0;
    const room = el.getBoundingClientRect().bottom - titlebar - 16;
    el.style.maxHeight = `${Math.max(160, Math.min(room, 400))}px`;
    if (el.getBoundingClientRect().left < 12) {
      el.dataset.side = "right";
    }
  }, [flyout]);

  // The detail card opens beside the popover, its top pinned to the active
  // row's top. It's portaled to the body (see render), so `offsetTop` is
  // meaningless — compute viewport-fixed coords here, the same math as
  // `showCatalogHover`. Prefer the left side; flip right only when the card
  // wouldn't fit on the left.
  useLayoutEffect(() => {
    if (flyout?.kind !== "model") {
      setDetailPos(null);
      return;
    }
    const popover = popoverRef.current;
    const row = popover?.querySelector<HTMLElement>(
      '.agent-composer-model-row[data-active="true"]',
    );
    if (!popover || !row) {
      setDetailPos(null);
      return;
    }
    const rowRect = row.getBoundingClientRect();
    const popoverRect = popover.getBoundingClientRect();
    const canOpenLeft =
      popoverRect.left -
        MODEL_HOVERCARD_GAP -
        MODEL_HOVERCARD_W -
        MODEL_HOVERCARD_VIEWPORT_MARGIN >=
      0;
    const canOpenRight =
      popoverRect.right +
        MODEL_HOVERCARD_GAP +
        MODEL_HOVERCARD_W +
        MODEL_HOVERCARD_VIEWPORT_MARGIN <=
      window.innerWidth;
    const side = canOpenLeft ? "left" : canOpenRight ? "right" : "left";
    setDetailPos({
      top: rowRect.top,
      x:
        side === "right"
          ? popoverRect.right + MODEL_HOVERCARD_GAP
          : popoverRect.left - MODEL_HOVERCARD_GAP,
      side,
    });
  }, [flyout, popoverRef]);

  // Keep the detail card on-screen: it's anchored to the active row's top, but
  // the picker opens downward, so a row near the viewport floor (or an expanded
  // description) would push the card off the bottom. Measure the real height
  // and pull it up so its bottom stays visible.
  useLayoutEffect(() => {
    if (!detailPos) return;
    const card = flyoutRef.current;
    if (!card) return;
    const height = card.getBoundingClientRect().height;
    if (height <= 0) return;
    const maxTop = window.innerHeight - height - MODEL_HOVERCARD_VIEWPORT_MARGIN;
    setDetailPos((prev) => {
      if (!prev) return prev;
      const clampedTop = Math.max(MODEL_HOVERCARD_VIEWPORT_MARGIN, Math.min(prev.top, maxTop));
      return Math.abs(clampedTop - prev.top) > 0.5 ? { ...prev, top: clampedTop } : prev;
    });
  }, [detailPos]);

  useLayoutEffect(() => {
    fade.update();
  }, [flyout, options, search, privateOnly, fade.update]);

  useEffect(() => {
    setCatalogHover(null);
  }, [flyout, search, privateOnly]);

  // Keep the row's fixed-positioned hover card inside the viewport vertically:
  // the card is anchored to the hovered row's top, but the settings picker
  // opens downward, so a row near the viewport floor would push the card off
  // the bottom edge. Measure the real card height and pull it up so its bottom
  // stays on-screen. Horizontal side is already clamped in showCatalogHover.
  useLayoutEffect(() => {
    if (!catalogHover) return;
    const card = hovercardRef.current;
    if (!card) return;
    const height = card.getBoundingClientRect().height;
    if (height <= 0) return;
    const maxTop = window.innerHeight - height - MODEL_HOVERCARD_VIEWPORT_MARGIN;
    setCatalogHover((prev) => {
      if (!prev) return prev;
      const clampedTop = Math.max(MODEL_HOVERCARD_VIEWPORT_MARGIN, Math.min(prev.top, maxTop));
      return Math.abs(clampedTop - prev.top) > 0.5 ? { ...prev, top: clampedTop } : prev;
    });
  }, [catalogHover]);

  const query = search.trim().toLowerCase();
  const selectable = useMemo(
    () => options.filter((option) => modelAvailableForMode(mode, option)),
    [mode, options],
  );
  const suggested = useMemo(() => {
    if (!suggestedModelIds) return suggestedModelsForMode(mode, selectable);
    return suggestedModelIds.flatMap((id, index) => {
      const option = selectable.find((candidate) => candidate.id === id);
      if (!option) return [];
      return [
        {
          key: `${id}:host:${index}`,
          model: option,
          reason: option.description ?? t("settingsPanels.picker.suggestedForSurface"),
        },
      ];
    });
  }, [mode, selectable, suggestedModelIds, t]);
  const privacyFiltered = privateOnly ? selectable.filter(modelIsPrivate) : selectable;
  const filteredOptions = query
    ? privacyFiltered.filter((option) => modelMatchesQuery(option, query))
    : privacyFiltered;

  // Root-layer search results: one flat list across both layers, curated
  // suggestions (with their preset preferences) leading, the rest of the
  // catalog following. Only alive on surfaces that opted into the root field.
  const rootListId = useId();
  const [rootActive, setRootActive] = useState(0);
  const rootListRef = useRef<HTMLDivElement | null>(null);
  const rootFade = useScrollFade(rootListRef);
  const rootQuery = (rootSearch ?? "").trim().toLowerCase();
  const rootQueryActive = Boolean(rootSearchRef) && Boolean(rootQuery);
  const rootMatchingSuggested = rootQueryActive
    ? suggested.filter(({ model: option }) => modelMatchesQuery(option, rootQuery))
    : [];
  const rootSuggestedIds = new Set(rootMatchingSuggested.map(({ model: option }) => option.id));
  const rootResults = rootQueryActive
    ? [
        ...rootMatchingSuggested.map(({ key, model: option }) => ({
          key,
          model: option,
        })),
        ...selectable
          .filter(
            (option) => !rootSuggestedIds.has(option.id) && modelMatchesQuery(option, rootQuery),
          )
          .map((option) => ({ key: option.id, model: option })),
      ]
    : [];
  const resolvedRootActive = Math.min(rootActive, Math.max(rootResults.length - 1, 0));
  const rootStatusId = `${rootListId}-status`;
  const matchingModelsStatus = rootResults.length
    ? t("settingsPanels.picker.matchingModels", { count: rootResults.length })
    : t("settingsPanels.picker.noResults");
  const rootSearchStatus = rootQueryActive ? matchingModelsStatus : undefined;
  useEffect(() => {
    setRootActive(0);
  }, [rootQuery]);
  useLayoutEffect(() => {
    if (rootQueryActive) rootFade.update();
  }, [rootQueryActive, rootResults.length, rootFade.update]);
  function moveRootActive(delta: number) {
    if (!rootResults.length) return;
    setRootActive((current) => {
      const currentIndex = Math.min(current, rootResults.length - 1);
      const nextIndex = (currentIndex + delta + rootResults.length) % rootResults.length;
      window.requestAnimationFrame(() => {
        document
          .getElementById(`${rootListId}-option-${nextIndex}`)
          ?.scrollIntoView?.({ block: "nearest" });
      });
      return nextIndex;
    });
  }
  function moveRootActiveTo(index: number) {
    if (!rootResults.length) return;
    const nextIndex = Math.max(0, Math.min(index, rootResults.length - 1));
    setRootActive(nextIndex);
    window.requestAnimationFrame(() => {
      document
        .getElementById(`${rootListId}-option-${nextIndex}`)
        ?.scrollIntoView?.({ block: "nearest" });
    });
  }
  const detail =
    flyout?.kind === "model" ? suggested.find((item) => item.key === flyout.id) : undefined;

  // Latest filtered rows, read by the hand-off closure without re-subscribing
  // the pointer listener on every keystroke.
  const filteredOptionsRef = useRef(filteredOptions);
  filteredOptionsRef.current = filteredOptions;

  const showCatalogHover = useCallback(
    (option: ProviderModelDto, row: HTMLElement) => {
      cancelCatalogClose();
      const panel = flyoutRef.current ?? popoverRef.current;
      if (!panel) return;
      const rowRect = row.getBoundingClientRect();
      const panelRect = panel.getBoundingClientRect();
      const preferred = panel.dataset.side === "right" ? "right" : "left";
      const canOpenLeft =
        panelRect.left -
          MODEL_HOVERCARD_GAP -
          MODEL_HOVERCARD_W -
          MODEL_HOVERCARD_VIEWPORT_MARGIN >=
        0;
      const canOpenRight =
        panelRect.right +
          MODEL_HOVERCARD_GAP +
          MODEL_HOVERCARD_W +
          MODEL_HOVERCARD_VIEWPORT_MARGIN <=
        window.innerWidth;
      const side =
        preferred === "left"
          ? canOpenLeft
            ? "left"
            : canOpenRight
              ? "right"
              : null
          : canOpenRight
            ? "right"
            : canOpenLeft
              ? "left"
              : null;
      if (!side) {
        setCatalogHover(null);
        return;
      }
      setCatalogHover({
        model: option,
        rowRect: rectFromElement(row),
        top: rowRect.top,
        x:
          side === "right"
            ? panelRect.right + MODEL_HOVERCARD_GAP
            : panelRect.left - MODEL_HOVERCARD_GAP,
        side,
      });
    },
    [cancelCatalogClose, popoverRef],
  );

  const resolveFilteredOption = useCallback(
    (id: string) => filteredOptionsRef.current.find((item) => item.id === id),
    [],
  );

  const modelBridge = useModelDetailHoverBridge({
    flyout,
    popoverRef,
    cardRef: flyoutRef,
    cancelHoverIntent,
    setBridging,
    onFlyoutChange,
  });

  const catalogBridge = useCatalogHoverBridge({
    catalogHover,
    cardRef: hovercardRef,
    listRef,
    resolveOption: resolveFilteredOption,
    showCatalogHover,
    cancelHoverIntent,
    cancelCatalogClose,
    scheduleCatalogClose,
    setBridging,
  });

  function handleListboxNavigation(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    const options = Array.from(
      event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="option"]'),
    );
    if (!options.length) return;
    event.preventDefault();
    const focusedIndex = options.indexOf(document.activeElement as HTMLButtonElement);
    const currentIndex = focusedIndex >= 0 ? focusedIndex : 0;
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? options.length - 1
          : (currentIndex + (event.key === "ArrowDown" ? 1 : -1) + options.length) % options.length;
    options[nextIndex]?.focus();
  }

  function catalogList(label: string) {
    return (
      <>
        <label className="agent-composer-model-search">
          <input
            ref={searchRef}
            value={search}
            onChange={(event) => onSearchChange(event.currentTarget.value)}
            placeholder={t("settingsPanels.models.search")}
            aria-label={t("settingsPanels.models.search")}
          />
        </label>
        <div className="agent-composer-model-filter">
          <span>{t("settingsPanels.models.private")}</span>
          <Switch
            checked={privateOnly}
            onCheckedChange={setPrivateOnly}
            aria-label={t("settingsPanels.picker.privateOnly")}
          />
        </div>
        <div className="agent-composer-model-list-wrap scroll-fade" {...fade.props}>
          <div
            ref={listRef}
            className="agent-composer-model-list"
            role="listbox"
            aria-label={label}
            onKeyDown={handleListboxNavigation}
            onScroll={() => {
              fade.update();
              cancelHoverIntent();
              setCatalogHover(null);
            }}
          >
            {filteredOptions.length ? (
              filteredOptions.map((option) => (
                <ModelPickerOption
                  key={option.id}
                  model={option}
                  selected={option.id === model?.id}
                  active={catalogHover?.model.id === option.id}
                  onSelect={onSelect}
                  onHover={(hoverModel, row, immediate) => {
                    if (!immediate && catalogBridge.isActive()) {
                      return;
                    }
                    cancelCatalogClose();
                    if (immediate || catalogHover) {
                      cancelHoverIntent();
                      showCatalogHover(hoverModel, row);
                    } else {
                      hoverIntent(() => showCatalogHover(hoverModel, row));
                    }
                  }}
                />
              ))
            ) : (
              <p className="agent-composer-model-empty" role="status" aria-live="polite">
                {privateOnly
                  ? query
                    ? t("settingsPanels.picker.noPrivateMatch")
                    : t("settingsPanels.picker.noPrivate")
                  : t("settingsPanels.picker.noModelsMatch")}
              </p>
            )}
          </div>
        </div>
      </>
    );
  }

  if (!model) return null;
  const rootResultsList = rootResults.length ? (
    <div
      className="agent-composer-model-list-wrap agent-composer-model-root-results scroll-fade"
      {...rootFade.props}
    >
      <div
        ref={rootListRef}
        id={rootListId}
        className="agent-composer-model-list"
        role="listbox"
        aria-label={t("settingsPanels.picker.matchingModelsAria")}
        onScroll={rootFade.update}
      >
        {rootResults.map(({ key, model: option }, index) => (
          <button
            key={key}
            id={`${rootListId}-option-${index}`}
            type="button"
            className="agent-composer-model-row"
            role="option"
            tabIndex={-1}
            aria-selected={option.id === model.id}
            data-active={index === resolvedRootActive || undefined}
            onPointerMove={() => setRootActive(index)}
            onClick={() => onSelect(option.id)}
          >
            <ModelPickerOptionText model={option} />
            {option.id === model.id ? (
              <IconCheckmark2Small
                size={14}
                aria-hidden
                className="agent-composer-model-row-check"
              />
            ) : null}
            <ModelRowPrivacyBadge model={option} />
          </button>
        ))}
      </div>
    </div>
  ) : (
    <p className="agent-composer-model-empty agent-composer-model-root-empty">
      {t("settingsPanels.picker.noResults")}
    </p>
  );
  return (
    <div
      ref={popoverRef}
      className={["agent-composer-model-popover", className].filter(Boolean).join(" ")}
      role="dialog"
      aria-label={ariaLabel}
      // Opening/closing the detail flyout is owned by the safe-polygon listener;
      // leaving the popover drops a not-yet-fired open intent and lifts any
      // bridging suppression left by an abandoned re-target, so row hover
      // feedback can never stay dead.
      onPointerLeave={() => {
        cancelHoverIntent();
        setBridging(false);
      }}
    >
      {rootSearchRef ? (
        <label className="agent-composer-model-search agent-composer-model-root-search">
          <input
            ref={rootSearchRef}
            value={rootSearch ?? ""}
            onChange={(event) => onRootSearchChange?.(event.currentTarget.value)}
            placeholder={t("settingsPanels.models.search")}
            aria-label={t("settingsPanels.models.search")}
            role="combobox"
            aria-autocomplete="list"
            aria-expanded={rootQueryActive && rootResults.length > 0}
            aria-controls={rootQueryActive && rootResults.length ? rootListId : undefined}
            aria-describedby={rootSearchStatus ? rootStatusId : undefined}
            aria-activedescendant={
              rootQueryActive && rootResults.length
                ? `${rootListId}-option-${resolvedRootActive}`
                : undefined
            }
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                event.stopPropagation();
                if (rootQueryActive && rootResults.length) {
                  const active = rootResults[resolvedRootActive];
                  onSelect(active.model.id);
                }
                return;
              }
              if (!rootQueryActive) return;
              if (event.key === "ArrowDown" && rootResults.length) {
                event.preventDefault();
                moveRootActive(1);
                return;
              }
              if (event.key === "ArrowUp" && rootResults.length) {
                event.preventDefault();
                moveRootActive(-1);
                return;
              }
              if (event.key === "Home" && rootResults.length) {
                event.preventDefault();
                moveRootActiveTo(0);
                return;
              }
              if (event.key === "End" && rootResults.length) {
                event.preventDefault();
                moveRootActiveTo(rootResults.length - 1);
                return;
              }
            }}
          />
        </label>
      ) : null}
      {rootSearchStatus ? (
        <span
          id={rootStatusId}
          className="visually-hidden"
          role="status"
          aria-live="polite"
          aria-atomic="true"
        >
          {rootSearchStatus}
        </span>
      ) : null}
      {rootQueryActive ? (
        rootResultsList
      ) : mode === "generation" ? (
        catalogList(allModelsLabel)
      ) : (
        <>
          <p className="agent-composer-model-title">{title}</p>
          <div
            className="agent-composer-model-menu"
            role="listbox"
            aria-label={suggestedListLabel}
            onKeyDown={handleListboxNavigation}
          >
            {suggested.length ? (
              suggested.map(({ key, model: option }) => (
                <button
                  key={key}
                  type="button"
                  className="agent-composer-model-row"
                  role="option"
                  aria-selected={option.id === model.id}
                  data-model-id={key}
                  data-active={(flyout?.kind === "model" && flyout.id === key) || undefined}
                  onMouseEnter={() => {
                    if (modelBridge.isActive()) {
                      return;
                    }
                    const open = () => onFlyoutChange({ kind: "model", id: key });
                    if (flyout) {
                      cancelHoverIntent();
                      open();
                    } else {
                      hoverIntent(open);
                    }
                  }}
                  onFocus={() => {
                    cancelHoverIntent();
                    onFlyoutChange({ kind: "model", id: key });
                  }}
                  onClick={() => onSelect(option.id)}
                >
                  <ModelPickerOptionText model={option} />
                  {option.id === model.id ? (
                    <IconCheckmark2Small
                      size={14}
                      aria-hidden
                      className="agent-composer-model-row-check"
                    />
                  ) : null}
                  <ModelRowPrivacyBadge model={option} />
                </button>
              ))
            ) : (
              <p className="agent-composer-model-empty">
                {t("settingsPanels.picker.loadingSuggested")}
              </p>
            )}
          </div>
          <button
            type="button"
            className="agent-composer-model-row agent-composer-model-all"
            aria-haspopup="true"
            aria-expanded={flyout?.kind === "all"}
            data-active={flyout?.kind === "all" || undefined}
            onMouseEnter={() => {
              if (modelBridge.isActive()) {
                return;
              }
              const open = () => onFlyoutChange({ kind: "all" });
              if (flyout) {
                cancelHoverIntent();
                open();
              } else {
                hoverIntent(open);
              }
            }}
            onFocus={() => {
              cancelHoverIntent();
              onFlyoutChange({ kind: "all" });
            }}
            onClick={() => {
              cancelHoverIntent();
              onFlyoutChange({ kind: "all" });
              searchRef.current?.focus();
            }}
          >
            <span className="agent-composer-model-row-name">
              {t("settingsPanels.picker.allModels")}
            </span>
            <IconChevronRightSmall
              size={16}
              aria-hidden
              className="agent-composer-model-row-chevron"
            />
          </button>
        </>
      )}
      {detail && portalTarget
        ? createPortal(
            // Portaled to the body and fixed-positioned so no scroll container
            // or panel (e.g. the note-chat panel) can clip it or paint over it.
            // Rendered as a .hovercard (position: fixed + z 140 + the slide
            // animation) rather than the absolute .flyout, but keeps
            // .agent-composer-model-detail for the card's own surface styles.
            <div
              ref={flyoutRef}
              className="agent-composer-model-hovercard agent-composer-model-detail"
              data-side={detailPos?.side ?? "left"}
              onPointerEnter={cancelHoverIntent}
              // Hidden for the one commit before the layout effect measures the
              // active row (which runs before paint, so no flash reaches screen).
              style={
                detailPos
                  ? detailPos.side === "right"
                    ? { top: detailPos.top, left: detailPos.x }
                    : { top: detailPos.top, right: window.innerWidth - detailPos.x }
                  : { visibility: "hidden" }
              }
            >
              <div className="agent-composer-model-surface">
                <ModelPickerCardContent model={detail.model} withDescription animateChange />
              </div>
            </div>,
            portalTarget,
          )
        : null}
      {flyout?.kind === "all" ? (
        <div
          ref={flyoutRef}
          className="agent-composer-model-flyout agent-composer-model-all-panel"
          role="group"
          aria-label={allModelsLabel}
          // Leaving the catalog panel abandons any pending re-target hover, so
          // also lift the bridging suppression here to keep row hover alive.
          onPointerLeave={() => {
            cancelHoverIntent();
            setBridging(false);
          }}
        >
          <div className="agent-composer-model-surface">{catalogList(allModelsLabel)}</div>
        </div>
      ) : null}
      {(mode === "generation" || flyout?.kind === "all") && catalogHover && portalTarget
        ? createPortal(
            // Portaled alongside the detail card, for the same reason.
            <div
              ref={hovercardRef}
              className="agent-composer-model-hovercard agent-composer-model-detail"
              data-side={catalogHover.side}
              onPointerEnter={cancelCatalogClose}
              style={
                catalogHover.side === "right"
                  ? { top: catalogHover.top, left: catalogHover.x }
                  : {
                      top: catalogHover.top,
                      right: window.innerWidth - catalogHover.x,
                    }
              }
            >
              <div className="agent-composer-model-surface">
                <ModelPickerCardContent model={catalogHover.model} withDescription animateChange />
              </div>
            </div>,
            portalTarget,
          )
        : null}
    </div>
  );
}

// The card: name + privacy chip, the full description in a capped scroll box,
// then the pricing/context facts as a compact spec list. The description never
// grows the card after it opens (no "Show more" toggle), so the open-time
// viewport clamp stays correct and there is no focusable control stranded
// inside the portaled subtree.
export function ModelPickerCardContent({
  model,
  withDescription,
  animateChange = false,
}: {
  model: ProviderModelDto;
  withDescription?: boolean;
  animateChange?: boolean;
}) {
  const badge = modelPrivacyBadge(model);
  const specs = modelSpecEntries(model);
  return (
    <div
      key={animateChange ? model.id : undefined}
      className="agent-composer-model-detail-content"
      data-animate-change={animateChange || undefined}
    >
      <p className="agent-composer-model-detail-name">
        <span>{model.name}</span>
        {badge ? (
          <ModelPrivacyChip
            badge={badge}
            withTip={false}
            variant="themed"
            size="sm"
            label={badge.label.replace(" mode", "")}
          />
        ) : null}
      </p>
      {withDescription && model.description ? (
        // Keyed by model so switching rows re-measures the fade from the top.
        <ModelCardDescription key={model.id} text={model.description} />
      ) : null}
      {specs.length ? (
        <dl className="agent-composer-model-detail-specs">
          {specs.map((spec) => (
            <Fragment key={spec.label}>
              <dt>{spec.label}</dt>
              <dd>{spec.value}</dd>
            </Fragment>
          ))}
        </dl>
      ) : null}
    </div>
  );
}

// Full description in a short capped box that scrolls when it overflows, with
// contained top/bottom scroll fades from the shared `useScrollFade` primitive
// (WKWebView-safe overlay flavor — never a mask on the scroller itself).
function ModelCardDescription({ text }: { text: string }) {
  const ref = useRef<HTMLParagraphElement | null>(null);
  const fade = useScrollFade(ref);
  return (
    <div className="agent-composer-model-detail-desc-wrap scroll-fade" {...fade.props}>
      <p ref={ref} className="agent-composer-model-detail-desc">
        {text}
      </p>
    </div>
  );
}

function ModelPickerOption({
  model,
  selected,
  active,
  onSelect,
  onHover,
}: {
  model: ProviderModelDto;
  selected: boolean;
  active?: boolean;
  onSelect: (modelId: string) => void;
  onHover: (model: ProviderModelDto, row: HTMLElement, immediate: boolean) => void;
}) {
  return (
    <button
      type="button"
      className="agent-composer-model-row"
      role="option"
      aria-selected={selected}
      data-model-id={model.id}
      data-active={active || undefined}
      onMouseEnter={(event) => onHover(model, event.currentTarget, false)}
      onFocus={(event) => onHover(model, event.currentTarget, true)}
      onClick={() => onSelect(model.id)}
    >
      <ModelPickerOptionText model={model} />
      {selected ? (
        <IconCheckmark2Small size={14} aria-hidden className="agent-composer-model-row-check" />
      ) : null}
      <ModelRowPrivacyBadge model={model} />
    </button>
  );
}

function ModelPickerOptionText({ model }: { model: ProviderModelDto }) {
  return (
    <>
      <span className="agent-composer-model-row-logo" aria-hidden>
        <ProviderLogo provider={model.provider} id={model.id} name={model.name} />
      </span>
      <span className="agent-composer-model-row-copy">
        <span className="agent-composer-model-row-name">{model.name}</span>
      </span>
    </>
  );
}
