import { IconArrowInbox } from "central-icons/IconArrowInbox";
import { type PointerEvent as ReactPointerEvent, useEffect, useRef, useState } from "react";
import { useT } from "../../../i18n";
import type { AgentChatPart } from "../../../lib/agent-chat-runtime";
import { agentFilePreview, localVideoFileSrc } from "../../../lib/tauri";
import { CLOVY_MARK_HEIGHT, CLOVY_MARK_PATH, CLOVY_MARK_WIDTH } from "../../brand/ClovyLogo";

const GENERATED_MEDIA_MARK_ROWS = 28;
const GENERATED_MEDIA_MARK_COLS = Math.round(
  GENERATED_MEDIA_MARK_ROWS * (CLOVY_MARK_WIDTH / CLOVY_MARK_HEIGHT),
);
const GENERATED_MEDIA_MARK_SAMPLES = [0.2, 0.5, 0.8] as const;

/* One shared parameter set so the two wave kinds stay in the same physical
 * register: a wavefront is a gaussian band that brightens dots and pushes
 * them away from its source; dots ease back as the band moves on. */
const GENERATED_MEDIA_FIELD = {
  pitch: 6,
  dotRadius: 1,
  markDotRadius: 1.25,
  markGlowGain: 1.2,
  maxAlpha: 0.85,
  /* The ambient sheen: a plane wavefront crossing left to right, both ends
   * fully off-canvas so the loop reset is invisible, then a rest beat. The
   * band leans at the shared shimmer utility's 20deg so the canvas sweep and
   * the label shimmer read as one system. */
  sweepCycleMs: 3600,
  sweepTravelMs: 2400,
  sweepSigma: 34,
  sweepPush: 2.2,
  sweepAngleDeg: 20,
  /* Pointer ripples: a radial wavefront expanding from the tap point. The
   * band also paints the dots it crosses with the theme accent. */
  ripplePxPerMs: 0.24,
  rippleSigma: 24,
  rippleTauMs: 950,
  ripplePush: 5,
  rippleGlow: 0.4,
  ripplePaintMix: 0.95,
  /* Mark sparkle: each logo dot glints on its own deterministic cadence - a
   * brief flash of clay brightness, never size. The glint is clay-tinted
   * (sparkMix) rather than gray so the mark reads as warm, but the tint only
   * lands clean because --brand-bright is a *luminous* clay (fixed high
   * lightness + healthy chroma); a duller white-mixed clay turns to mud over
   * the light dot field. The pulse uses a near-instant attack and a longer
   * release, matching the clean snap of a light catching an edge instead of a
   * soft sine-wave throb. The staggered cadence keeps the mark alive without
   * making every dot pulse at once; the press ripple keeps the fuller accent
   * burst (ripplePaintMix) for a deliberate tap. */
  sparkMinRadPerSec: 1.6,
  sparkSpanRadPerSec: 1.2,
  sparkAttackRatio: 0.025,
  sparkDecayRatio: 0.1,
  sparkMix: 0.72,
  sparkAlphaBoost: 0.52,
  /* The dot field thins out over this many px at the canvas bottom, into the
   * card-surface gradient the CSS background lands on. */
  bottomFadePx: 56,
};

type GeneratedMediaRipple = { x: number; y: number; startedAt: number };

/** The particle dot field behind a generating image/video: a fine stationary
 * lattice carrying the Clovy mark as brighter dots, with a soft sheen
 * wavefront sweeping across on a fixed cadence. Pointer taps drop radial
 * ripples that push dots outward and let them settle back. Dot positions are
 * a pure function of time (no per-dot state), so dropped frames never desync
 * the motion; reduced motion renders a single static frame. */
function GeneratedMediaDotField() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const ripplesRef = useRef<GeneratedMediaRipple[]>([]);
  const reducedMotionRef = useRef(false);

  useEffect(() => {
    const F = GENERATED_MEDIA_FIELD;
    const canvas = canvasRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context) return; // test env has no 2d context

    let width = 0;
    let height = 0;
    // `mark` is the glyph-coverage weight of this lattice cell, 0..1; the
    // spark fields give each logo dot its own deterministic glint cadence.
    let dots: Array<{
      x: number;
      y: number;
      mark: number;
      sparkOmega: number;
      sparkPhase: number;
    }> = [];
    let raf = 0;

    /* The ink colors and per-theme alphas live in CSS so the field follows
     * the design tokens; the canvas reads their computed values. The theme
     * accent rides in through `accent-color`, which computes to a concrete
     * color without painting anything on a canvas element. */
    const readInk = () => {
      const style = getComputedStyle(canvas);
      const accent = style.accentColor;
      return {
        color: style.color,
        spark: accent && accent !== "auto" ? accent : style.color,
        dotAlpha: Number.parseFloat(style.getPropertyValue("--agent-generated-dot-alpha")) || 0.08,
        sheenGlow:
          Number.parseFloat(style.getPropertyValue("--agent-generated-sheen-glow")) || 0.24,
        markAlpha:
          Number.parseFloat(style.getPropertyValue("--agent-generated-mark-alpha")) || 0.32,
      };
    };
    let ink = readInk();

    const rebuild = () => {
      const rect = canvas.getBoundingClientRect();
      width = Math.round(rect.width);
      height = Math.round(rect.height);
      const dpr = window.devicePixelRatio || 1;
      canvas.width = Math.max(1, Math.round(width * dpr));
      canvas.height = Math.max(1, Math.round(height * dpr));
      context.setTransform(dpr, 0, 0, dpr, 0, 0);
      const cols = Math.ceil(width / F.pitch);
      const rows = Math.ceil(height / F.pitch);
      const markPath = new Path2D(CLOVY_MARK_PATH);
      const markCols = GENERATED_MEDIA_MARK_COLS;
      const markRows = GENERATED_MEDIA_MARK_ROWS;
      // Centered on the lattice, lifted one row to balance the footer bar.
      const markCol = Math.round((cols - markCols) / 2);
      const markRow = Math.round((rows - markRows) / 2) - 1;
      dots = [];
      for (let row = 0; row < rows; row++) {
        for (let col = 0; col < cols; col++) {
          const inMarkBounds =
            row >= markRow &&
            row < markRow + markRows &&
            col >= markCol &&
            col < markCol + markCols;
          let mark = 0;
          if (inMarkBounds) {
            const localRow = row - markRow;
            const localCol = col - markCol;
            let covered = 0;
            for (const sampleY of GENERATED_MEDIA_MARK_SAMPLES) {
              for (const sampleX of GENERATED_MEDIA_MARK_SAMPLES) {
                const x = ((localCol + sampleX) / markCols) * CLOVY_MARK_WIDTH;
                const y = ((localRow + sampleY) / markRows) * CLOVY_MARK_HEIGHT;
                if (context.isPointInPath(markPath, x, y, "evenodd")) covered += 1;
              }
            }
            mark = covered / GENERATED_MEDIA_MARK_SAMPLES.length ** 2;
          }
          // Two lattice-position hashes decorrelate each dot's glint cycle.
          const noise = Math.sin((row * 131 + col) * 12.9898) * 43758.5453;
          const seed = noise - Math.floor(noise);
          const noise2 = Math.sin((row * 131 + col) * 78.233) * 12543.8567;
          const seed2 = noise2 - Math.floor(noise2);
          dots.push({
            x: col * F.pitch + F.pitch / 2,
            y: row * F.pitch + F.pitch / 2,
            mark,
            sparkOmega: F.sparkMinRadPerSec + seed * F.sparkSpanRadPerSec,
            sparkPhase: seed2 * Math.PI * 2,
          });
        }
      }
    };

    const epoch = performance.now();
    // The sweep axis: dots are banded by their projection onto this direction.
    const sweepCos = Math.cos((F.sweepAngleDeg * Math.PI) / 180);
    const sweepSin = Math.sin((F.sweepAngleDeg * Math.PI) / 180);

    const draw = (t: number, animated: boolean) => {
      const ripples = ripplesRef.current;
      for (let i = ripples.length - 1; i >= 0; i--) {
        if (t - ripples[i].startedAt > 6 * F.rippleTauMs) ripples.splice(i, 1);
      }
      let front: number | null = null;
      if (animated) {
        const phase = ((t - epoch) % F.sweepCycleMs) / F.sweepTravelMs;
        const span = width * sweepCos + height * sweepSin;
        if (phase <= 1) front = -3 * F.sweepSigma + phase * (span + 6 * F.sweepSigma);
      }
      context.clearRect(0, 0, width, height);
      let fill = ink.color;
      context.fillStyle = fill;
      const setFill = (color: string) => {
        if (color !== fill) {
          fill = color;
          context.fillStyle = color;
        }
      };
      const seconds = t / 1000;
      for (const dot of dots) {
        let glow = 0;
        let paint = 0;
        let dx = 0;
        let dy = 0;
        if (front !== null) {
          const along = dot.x * sweepCos + dot.y * sweepSin;
          const band = Math.exp(-((along - front) ** 2) / (2 * F.sweepSigma ** 2));
          glow += ink.sheenGlow * band;
          dx += F.sweepPush * band * sweepCos;
          dy += F.sweepPush * band * sweepSin;
        }
        for (const ripple of ripples) {
          const age = t - ripple.startedAt;
          if (age < 0) continue;
          const rx = dot.x - ripple.x;
          const ry = dot.y - ripple.y;
          const dist = Math.hypot(rx, ry) || 1;
          const band =
            Math.exp(-((dist - F.ripplePxPerMs * age) ** 2) / (2 * F.rippleSigma ** 2)) *
            Math.exp(-age / F.rippleTauMs);
          glow += F.rippleGlow * band;
          paint += band;
          dx += (rx / dist) * F.ripplePush * band;
          dy += (ry / dist) * F.ripplePush * band;
        }
        // The glint: a quick accent strike with a slightly longer fade, out of
        // each logo dot's staggered cycle.
        let spark = 0;
        if (animated && dot.mark > 0) {
          const cycle =
            ((seconds * dot.sparkOmega + dot.sparkPhase) % (Math.PI * 2)) / (Math.PI * 2);
          if (cycle < F.sparkAttackRatio) {
            const progress = cycle / F.sparkAttackRatio;
            spark = progress * progress * (3 - 2 * progress);
          } else if (cycle < F.sparkAttackRatio + F.sparkDecayRatio) {
            const progress = (cycle - F.sparkAttackRatio) / F.sparkDecayRatio;
            spark = 1 - progress * progress * (3 - 2 * progress);
          }
          spark *= dot.mark;
        }
        // Partial glyph coverage blends the dot between field and mark, so
        // the mark's rounded corners and bevels stay soft on the lattice.
        const base = ink.dotAlpha + (ink.markAlpha - ink.dotAlpha) * dot.mark;
        const gain = 1 + (F.markGlowGain - 1) * dot.mark;
        // Thin the field out where the canvas background gradates into the
        // card surface, so the grid gives way instead of hitting an edge.
        const edge = Math.min(1, (height - dot.y) / F.bottomFadePx);
        const bottomFade = edge * edge * (3 - 2 * edge);
        const alpha =
          Math.min(F.maxAlpha, base + glow * gain + spark * F.sparkAlphaBoost) * bottomFade;
        const radius = F.dotRadius + (F.markDotRadius - F.dotRadius) * dot.mark;
        // How much of the dot's paint comes from the theme accent: the glint
        // plus the ripple's burst of color from a press.
        const mix = Math.min(0.95, spark * F.sparkMix + paint * F.ripplePaintMix);
        const x = dot.x + dx;
        const y = dot.y + dy;
        if (mix > 0.02) {
          setFill(ink.spark);
          context.globalAlpha = alpha * mix;
          context.beginPath();
          context.arc(x, y, radius, 0, Math.PI * 2);
          context.fill();
        }
        setFill(ink.color);
        context.globalAlpha = alpha * (1 - mix);
        context.beginPath();
        context.arc(x, y, radius, 0, Math.PI * 2);
        context.fill();
      }
      context.globalAlpha = 1;
    };

    const frame = () => {
      raf = requestAnimationFrame(frame);
      draw(performance.now(), true);
    };
    const stop = () => {
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
    };

    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const applyMotionPreference = () => {
      reducedMotionRef.current = reducedMotion.matches;
      stop();
      if (reducedMotion.matches) {
        ripplesRef.current = [];
        draw(performance.now(), false);
      } else {
        raf = requestAnimationFrame(frame);
      }
    };

    rebuild();
    // The generation "lands" with one ripple from the center of the canvas.
    ripplesRef.current = [{ x: width / 2, y: height / 2, startedAt: epoch + 50 }];
    applyMotionPreference();
    reducedMotion.addEventListener("change", applyMotionPreference);

    const resizeObserver = new ResizeObserver(() => {
      rebuild();
      if (reducedMotionRef.current) draw(performance.now(), false);
    });
    resizeObserver.observe(canvas);

    // Theme flips swap the computed ink; repaint with the new values.
    const themeObserver = new MutationObserver(() => {
      ink = readInk();
      if (reducedMotionRef.current) draw(performance.now(), false);
    });
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });

    return () => {
      stop();
      reducedMotion.removeEventListener("change", applyMotionPreference);
      resizeObserver.disconnect();
      themeObserver.disconnect();
    };
  }, []);

  const handlePointerDown = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (reducedMotionRef.current) return;
    const rect = event.currentTarget.getBoundingClientRect();
    ripplesRef.current.push({
      x: event.clientX - rect.left,
      y: event.clientY - rect.top,
      startedAt: performance.now(),
    });
  };

  return (
    <canvas
      ref={canvasRef}
      className="agent-generated-media-field"
      onPointerDown={handlePointerDown}
    />
  );
}

/** A quiet particle dot-field canvas carrying the Clovy mark, with
 * its working label in a separate footer. */
function AgentGeneratedMediaPlaceholder({ kind }: { kind: "image" | "video" }) {
  const t = useT();
  const label =
    kind === "image"
      ? t("chat.media.generatingImageEllipsis")
      : t("chat.media.generatingVideoEllipsis");
  return (
    <div className="agent-generated-media-placeholder-card">
      <div className={`agent-generated-${kind}-placeholder`} aria-hidden>
        <GeneratedMediaDotField />
      </div>
      <div className="agent-generated-media-status-bar">
        <span className="agent-generated-media-label text-shimmer shimmer">{label}</span>
      </div>
    </div>
  );
}

/** Completion reveal for generated media: when a watched running turn
 * completes and its bytes are ready, the media develops out of the generating
 * field - the dot-field surface mounts over it (its entrance ripple doubling
 * as the completion burst) and dissolves. Arming on the running -> complete
 * flip keeps history loads and reduced motion on the instant swap. */
function useGeneratedMediaReveal(status: "running" | "complete" | "error", ready: boolean) {
  const [revealing, setRevealing] = useState(false);
  const armedRef = useRef(false);
  const prevStatusRef = useRef(status);
  useEffect(() => {
    if (prevStatusRef.current === "running" && status === "complete") {
      armedRef.current = !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    }
    prevStatusRef.current = status;
  }, [status]);
  useEffect(() => {
    if (!armedRef.current || status !== "complete" || !ready) return;
    armedRef.current = false;
    setRevealing(true);
    const timer = setTimeout(() => setRevealing(false), 900);
    return () => clearTimeout(timer);
  }, [status, ready]);
  return revealing;
}

export function AgentGeneratedImage({
  part,
  onOpen,
  onDownload,
  onRetry,
}: {
  part: Extract<AgentChatPart, { type: "image" }>;
  onOpen?: (part: Extract<AgentChatPart, { type: "image" }>) => void;
  onDownload?: (part: Extract<AgentChatPart, { type: "image" }>) => void;
  onRetry?: () => void;
}) {
  const t = useT();
  const [pathPreviewDataUrl, setPathPreviewDataUrl] = useState<string | null>(null);

  useEffect(() => {
    if (part.status !== "complete" || part.dataUrl || !part.path) {
      setPathPreviewDataUrl(null);
      return;
    }
    let cancelled = false;
    setPathPreviewDataUrl(null);
    agentFilePreview(part.path)
      .then((dataUrl) => {
        if (!cancelled) setPathPreviewDataUrl(dataUrl);
      })
      .catch(() => {
        if (!cancelled) setPathPreviewDataUrl(null);
      });
    return () => {
      cancelled = true;
    };
  }, [part.status, part.dataUrl, part.path]);

  const imageSrc = part.dataUrl ?? pathPreviewDataUrl;
  const revealing = useGeneratedMediaReveal(part.status, Boolean(imageSrc));

  if (part.status === "running") {
    return (
      <div
        className="agent-generated-image"
        data-status="running"
        role="status"
        aria-label={t("chat.media.generatingImage")}
        aria-live="polite"
      >
        <AgentGeneratedMediaPlaceholder kind="image" />
      </div>
    );
  }
  if (part.status === "error") {
    return (
      <div className="agent-generated-image" data-status="error">
        <p className="agent-generated-image-error">
          {part.error?.trim() || t("chat.media.imageFailed")}
        </p>
        {onRetry && part.requestId ? (
          <button type="button" className="agent-generated-image-retry" onClick={onRetry}>
            {t("common.tryAgain")}
          </button>
        ) : null}
      </div>
    );
  }
  const label = part.name?.trim() || t("chat.media.generatedImage");
  // "Open" enlarges filesystem-backed images in the artifact viewer. MCP image
  // blocks have only inline bytes, so they render as a plain frame. Legacy
  // MEDIA references have a path and lazily fetch their preview data url above.
  const image = imageSrc ? (
    <img src={imageSrc} alt={part.prompt} draggable={false} />
  ) : part.path ? (
    <span className="agent-generated-image-loading text-shimmer shimmer">
      {t("chat.media.loadingImage")}
    </span>
  ) : null;
  const reveal = revealing ? (
    <span className="agent-generated-media-reveal" aria-hidden>
      <GeneratedMediaDotField />
    </span>
  ) : null;
  return (
    <figure
      className="agent-generated-image"
      data-status="complete"
      data-revealing={revealing ? "true" : undefined}
    >
      {part.path ? (
        <button
          type="button"
          className="agent-generated-image-frame"
          onClick={() => onOpen?.(part)}
          aria-label={t("chat.openNamed", { name: label })}
          title={t("chat.media.openImage")}
        >
          {image}
          {reveal}
        </button>
      ) : (
        <div className="agent-generated-image-frame">
          {image}
          {reveal}
        </div>
      )}
      <figcaption className="agent-generated-image-bar">
        <span className="agent-generated-image-name" title={label}>
          {label}
        </span>
        {onDownload ? (
          <button
            type="button"
            className="agent-generated-image-download"
            onClick={() => onDownload(part)}
            aria-label={t("chat.media.downloadImage")}
            title={t("chat.media.downloadImage")}
          >
            <IconArrowInbox size={15} aria-hidden />
            <span>{t("chat.media.download")}</span>
          </button>
        ) : null}
      </figcaption>
    </figure>
  );
}

export function AgentGeneratedVideo({
  part,
  onDownload,
  onRetry,
}: {
  part: Extract<AgentChatPart, { type: "video" }>;
  onDownload?: (part: Extract<AgentChatPart, { type: "video" }>) => void;
  onRetry?: () => void;
}) {
  const t = useT();
  const src = part.status === "complete" && part.path ? localVideoFileSrc(part.path) : undefined;
  const [capturedPoster, setCapturedPoster] = useState<{ src: string; dataUrl: string }>();
  const poster =
    part.posterDataUrl ??
    (capturedPoster && capturedPoster.src === src ? capturedPoster.dataUrl : undefined);
  const revealing = useGeneratedMediaReveal(part.status, Boolean(src));

  useEffect(() => {
    // Capture the poster off an offscreen element so the visible player can stay
    // in no-CORS mode: the asset protocol omits `Access-Control-Allow-Origin` on
    // 416 range responses, and only the canvas capture needs CORS.
    if (!src || part.posterDataUrl || poster) return;
    let mounted = true;
    void capturedGeneratedVideoPoster(src).then((dataUrl) => {
      if (mounted && dataUrl) setCapturedPoster({ src, dataUrl });
    });
    return () => {
      mounted = false;
    };
  }, [src, part.posterDataUrl, poster]);

  if (part.status === "running") {
    return (
      <div
        className="agent-generated-video"
        data-status="running"
        role="status"
        aria-label={t("chat.media.generatingVideo")}
        aria-live="polite"
      >
        <AgentGeneratedMediaPlaceholder kind="video" />
      </div>
    );
  }
  if (part.status === "error") {
    return (
      <div className="agent-generated-video" data-status="error">
        <p className="agent-generated-image-error">
          {part.error?.trim() || t("chat.media.videoFailed")}
        </p>
        {onRetry && part.requestId ? (
          <button type="button" className="agent-generated-image-retry" onClick={onRetry}>
            {t("common.tryAgain")}
          </button>
        ) : null}
      </div>
    );
  }
  const label = part.name?.trim() || t("chat.media.generatedVideo");
  return (
    <figure
      className="agent-generated-video"
      data-status="complete"
      data-revealing={revealing ? "true" : undefined}
    >
      <div className="agent-generated-video-frame">
        {src ? (
          <video controls src={firstFrameVideoSource(src)} poster={poster} preload="metadata" />
        ) : (
          <span className="agent-generated-image-loading text-shimmer shimmer">
            {t("chat.media.loadingVideo")}
          </span>
        )}
        {revealing ? (
          <span className="agent-generated-media-reveal" aria-hidden>
            <GeneratedMediaDotField />
          </span>
        ) : null}
      </div>
      <figcaption className="agent-generated-image-bar">
        <span className="agent-generated-image-name" title={label}>
          {label}
        </span>
        {onDownload && part.path ? (
          <button
            type="button"
            className="agent-generated-image-download"
            onClick={() => onDownload(part)}
            aria-label={t("chat.media.downloadVideo")}
            title={t("chat.media.downloadVideo")}
          >
            <IconArrowInbox size={15} aria-hidden />
            <span>{t("chat.media.download")}</span>
          </button>
        ) : null}
      </figcaption>
    </figure>
  );
}

function firstFrameVideoSource(src: string) {
  return src.includes("#") ? src : `${src}#t=0.001`;
}

// Poster capture is CORS-mode work (canvas.toDataURL taints without it), so it
// runs on a throwaway offscreen element rather than the visible player. Cache
// the in-flight promise per src so the capture runs at most once per app run,
// even across remounts.
const generatedVideoPosterCache = new Map<string, Promise<string | undefined>>();

function capturedGeneratedVideoPoster(src: string): Promise<string | undefined> {
  const cached = generatedVideoPosterCache.get(src);
  if (cached) return cached;
  const capture = new Promise<string | undefined>((resolve) => {
    const video = document.createElement("video");
    video.crossOrigin = "anonymous";
    video.preload = "auto";
    video.muted = true;
    const finish = (dataUrl?: string) => {
      video.removeAttribute("src");
      video.load();
      resolve(dataUrl);
    };
    video.addEventListener("loadeddata", () => finish(firstFramePosterDataUrl(video)), {
      once: true,
    });
    video.addEventListener("error", () => finish(), { once: true });
    video.src = firstFrameVideoSource(src);
  });
  generatedVideoPosterCache.set(src, capture);
  return capture;
}

function firstFramePosterDataUrl(video: HTMLVideoElement): string | undefined {
  if (!video.videoWidth || !video.videoHeight) return undefined;
  const scale = Math.min(1, 960 / Math.max(video.videoWidth, video.videoHeight));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(video.videoWidth * scale));
  canvas.height = Math.max(1, Math.round(video.videoHeight * scale));
  const context = canvas.getContext("2d");
  if (!context) return undefined;
  try {
    context.drawImage(video, 0, 0, canvas.width, canvas.height);
    return canvas.toDataURL("image/jpeg", 0.82);
  } catch {
    // Asset-protocol or codec restrictions should never block video playback.
    return undefined;
  }
}

/** A resolved action card renders as a quiet, expandable one-line row instead
 * of a full card — a receipt in the transcript rather than a prompt. The row
 * mirrors {@link ContextCompactionPart}: an outcome glyph (checkmark / cross /
 * warning)
 * that cross-fades to a plain-text "+"/"−" on hover (pure opacity — no layout
 * shift, a WKWebView compositing constraint), a short outcome label, and a
 * truncated one-line detail. Expanding reveals the full detail body (the
 * `children`) minus the action buttons. */
