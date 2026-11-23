import type { BackgroundHandle } from "asciify-engine/kit";
import { useEffect, useRef, useState } from "react";
import { readBackgroundImage, useAppBackground } from "../../lib/app-background";

// One strength for both themes: the field is a faint texture behind the content.
const DRIFT_OPACITY = 0.14;

export function AgentBackground() {
  const preference = useAppBackground();
  return preference === "drift" ? (
    <DriftBackground />
  ) : preference === "image" ? (
    <ImageBackground />
  ) : null;
}

function DriftBackground() {
  const hostRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let disposed = false;
    let handle: BackgroundHandle | undefined;
    void import("asciify-engine/kit")
      .then(({ background, drift }) => {
        if (disposed) return;
        const mounted = background(host, {
          preset: drift,
          mark: "dot",
          ink: "--ascii-ink",
          speed: 1,
          cellSize: 11,
          opacity: DRIFT_OPACITY,
          cursor: "trail",
          look: { scene: "flow", contrast: 1.4, scale: 1.6 },
        });
        handle = mounted;
      })
      .catch(() => {
        // Decoration is optional: keep the content on the plain app surface.
      });
    return () => {
      disposed = true;
      handle?.destroy();
    };
  }, []);
  return <div ref={hostRef} className="agent-background" aria-hidden="true" />;
}

function ImageBackground() {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let disposed = false;
    void readBackgroundImage()
      .then((image) => {
        if (!disposed) setSrc(image?.dataUrl ?? null);
      })
      .catch(() => {});
    return () => {
      disposed = true;
    };
  }, []);
  if (!src) return null;
  return (
    <div className="agent-background agent-image-background" aria-hidden="true">
      <img src={src} alt="" onError={() => setSrc(null)} />
    </div>
  );
}
