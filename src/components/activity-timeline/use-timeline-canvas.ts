import { useLayoutEffect, useState } from "react";
import { FONT_SCALE_PRESETS, useFontScaleId } from "../../lib/font-scale";

/**
 * The workstream timeline's usable canvas: the scroller's content box (never
 * the canvas whose height the layout sets), the minimum card height and gap
 * resolved from the interface, and the text-size preference that scales the
 * card density thresholds.
 */
export function useTimelineCanvas() {
  const [element, ref] = useState<HTMLDivElement | null>(null);
  const [size, setSize] = useState({ canvasHeight: 0, minCardHeight: 22, cardGap: 6 });
  const fontScale = useFontScaleId();
  const textScale = FONT_SCALE_PRESETS.find((preset) => preset.id === fontScale)?.value ?? 1;

  useLayoutEffect(() => {
    if (!element) return;
    const measure = () => {
      const style = getComputedStyle(element);
      const canvasHeight = Math.max(
        0,
        element.clientHeight -
          (Number.parseFloat(style.paddingTop) || 0) -
          (Number.parseFloat(style.paddingBottom) || 0),
      );
      const minCardHeight = Math.ceil(
        Math.max(
          Number.parseFloat(style.getPropertyValue("--control-xs")) || 22,
          (Number.parseFloat(style.fontSize) || 13 * textScale) * 1.25 + 2,
        ),
      );
      const cardGap = Number.parseFloat(style.getPropertyValue("--sp-2")) || 6;
      setSize((previous) =>
        previous.canvasHeight === canvasHeight &&
        previous.minCardHeight === minCardHeight &&
        previous.cardGap === cardGap
          ? previous
          : { canvasHeight, minCardHeight, cardGap },
      );
    };
    // A text-size change needs a new line-box measurement even when the
    // scroller's outer dimensions stay unchanged.
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [element, textScale]);

  return { ref, textScale, ...size };
}
