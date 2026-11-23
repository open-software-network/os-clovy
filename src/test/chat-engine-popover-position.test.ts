import { describe, expect, it } from "vitest";
import { chatEnginePopoverPosition } from "../components/agent/composer/chat-engine-popover-position";

const viewport = {
  viewportWidth: 1200,
  viewportHeight: 800,
  topInset: 28,
  gap: 4,
  viewportMargin: 12,
};

function rect({
  left,
  top,
  width,
  height,
}: {
  left: number;
  top: number;
  width: number;
  height: number;
}) {
  return {
    left,
    right: left + width,
    top,
    bottom: top + height,
    width,
    height,
  } as DOMRect;
}

describe("chatEnginePopoverPosition", () => {
  it("right-aligns above the trigger when the menu fits", () => {
    const result = chatEnginePopoverPosition({
      triggerRect: rect({ left: 900, top: 700, width: 100, height: 36 }),
      anchorRect: rect({ left: 100, top: 620, width: 1000, height: 140 }),
      popoverRect: rect({ left: 0, top: 0, width: 232, height: 320 }),
      ...viewport,
    });

    expect(result).toEqual({
      placement: "above",
      left: 668,
      bottom: 64,
      maxHeight: 656,
    });
  });

  it("flips below the trigger when the menu does not fit above", () => {
    const result = chatEnginePopoverPosition({
      triggerRect: rect({ left: 900, top: 80, width: 100, height: 36 }),
      anchorRect: rect({ left: 100, top: 40, width: 1000, height: 140 }),
      popoverRect: rect({ left: 0, top: 0, width: 232, height: 320 }),
      ...viewport,
    });

    expect(result).toEqual({
      placement: "below",
      left: 668,
      top: 80,
      maxHeight: 668,
    });
  });

  it("uses the roomier side and caps the menu when neither side fits", () => {
    const result = chatEnginePopoverPosition({
      triggerRect: rect({ left: 900, top: 330, width: 100, height: 36 }),
      anchorRect: rect({ left: 100, top: 280, width: 1000, height: 140 }),
      popoverRect: rect({ left: 0, top: 0, width: 232, height: 600 }),
      ...viewport,
    });

    expect(result).toEqual({
      placement: "below",
      left: 668,
      top: 90,
      maxHeight: 418,
    });
  });

  it("keeps the menu inside the viewport when the trigger hugs an edge", () => {
    const result = chatEnginePopoverPosition({
      triggerRect: rect({ left: 8, top: 700, width: 100, height: 36 }),
      anchorRect: rect({ left: 0, top: 620, width: 1100, height: 140 }),
      popoverRect: rect({ left: 0, top: 0, width: 232, height: 320 }),
      ...viewport,
    });

    expect(result.left).toBe(12);
  });
});
