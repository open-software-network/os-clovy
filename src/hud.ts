import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { IconWindow } from "central-icons/IconWindow";
import { activityAppIcon } from "./lib/activity-timeline";
import { detectMeetingWindow, type DictationTarget, type MeetingWindowDto } from "./lib/tauri";
import { isMacLikePlatform } from "./lib/platform";
import { IconCrossMedium } from "central-icons/IconCrossMedium";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconExclamationCircle } from "central-icons/IconExclamationCircle";
import { IconMicrophone } from "central-icons-filled/IconMicrophone";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  clamp,
  createBarMeter,
  HUD_BAR_HISTORY_OFFSETS,
  HUD_BAR_WEIGHTS,
  IDLE_PULSE_AMP,
  IDLE_LEVEL,
  LIVE_WAVE_OPTIONS,
  withWaveLayers,
} from "./lib/audio-meter";
import { SPINNER_GRID_COLS, spinnerGrid } from "./lib/spinner-grid";
import { isOnboardingComplete, subscribeToOnboardingComplete } from "./lib/onboarding";
import { installNativeContextMenuGuard } from "./lib/native-context-menu";
import { subscribeBrand } from "./lib/brand";
import { messageForErrorCode } from "./lib/errors";
import { createHudLifecycle } from "./lib/hud-lifecycle";
import { subscribeInterfaceLocale, subscribeInterfaceLocaleAcrossWindows, t } from "./i18n";
import "./styles/hud.css";

const lifecycle = createHudLifecycle();

// Recolor this HUD window to the selected accent and keep it live-synced.
lifecycle.trackUnlisten(subscribeBrand());
// Follow the interface language chosen in the main window (sets <html lang>).
lifecycle.addCleanup(subscribeInterfaceLocaleAcrossWindows());

type DictationHudEvent = {
  type: string;
  payload?: {
    app?: string;
    target?: DictationTarget | null;
    code?: string;
    message?: string;
    level?: string;
    [key: string]: unknown;
  };
};

// Absent on the standalone browser page (no Tauri bridge), where the demo
// driver exercises the pill — getCurrentWindow() throws there.
const appWindow = (() => {
  try {
    return getCurrentWindow();
  } catch {
    return undefined;
  }
})();

lifecycle.addCleanup(installNativeContextMenuGuard());

const hud = document.querySelector<HTMLDivElement>("#hud");
const targetNode = document.querySelector<HTMLElement>("#hud-target");
const targetImage = targetNode?.querySelector<HTMLImageElement>("img");
const targetFallback = targetNode?.querySelector<HTMLElement>(".hud-target-fallback");
let targetName: string | undefined;
let targetIdentity: string | undefined;
let targetGeneration = 0;
let targetPinned = false;
let targetActive = false;

function renderTargetLabel() {
  const label = targetName
    ? t("hud.dictation.target", { app: targetName })
    : t("hud.dictation.targetUnknown");
  targetNode?.setAttribute("aria-label", label);
  targetNode?.setAttribute("title", label);
}

function clearTargetImage() {
  if (targetImage) {
    targetImage.hidden = true;
    targetImage.removeAttribute("src");
  }
  if (targetFallback) targetFallback.hidden = false;
}

function resetTarget() {
  targetGeneration += 1;
  targetName = undefined;
  targetIdentity = undefined;
  targetPinned = false;
  targetActive = false;
  if (targetNode) targetNode.hidden = true;
  clearTargetImage();
  renderTargetLabel();
}

function updateTarget(value: unknown) {
  const metadata =
    value && typeof value === "object" ? (value as Record<string, unknown>) : undefined;
  const name = typeof metadata?.app === "string" ? metadata.app.trim() : "";
  const bundleId = typeof metadata?.bundleIdentifier === "string" ? metadata.bundleIdentifier : "";
  const identity = JSON.stringify([name, bundleId, metadata?.processIdentifier]);
  targetName = name || undefined;
  renderTargetLabel();
  if (identity === targetIdentity) return;
  targetIdentity = identity;
  const generation = ++targetGeneration;
  clearTargetImage();
  if (!name || !bundleId) return;
  void activityAppIcon(bundleId)
    .then((icon) => {
      if (lifecycle.signal.aborted || generation !== targetGeneration || !targetActive) return;
      // The shared resolver returns only bounded, local PNGs. Never load a URL
      // from helper metadata, even if a malformed event supplies one.
      if (!icon?.startsWith("data:image/png;base64,") || !targetImage) return;
      targetImage.src = icon;
      targetImage.hidden = false;
      if (targetFallback) targetFallback.hidden = true;
    })
    .catch(() => {});
}

targetImage?.addEventListener("error", clearTargetImage);
if (targetFallback) {
  targetFallback.innerHTML = renderToStaticMarkup(
    createElement(IconWindow, { size: 18, ariaHidden: true, focusable: false }),
  );
}
lifecycle.addCleanup(resetTarget);

const bars = Array.from(document.querySelectorAll<HTMLElement>(".hud-bar"));
const spinnerNode = document.querySelector<HTMLElement>("#hud-spinner");
const errorText = document.querySelector<HTMLElement>("#hud-error-text");
const errorIcon = document.querySelector<HTMLElement>(".hud-error-icon");
const errorLayer = document.querySelector<HTMLElement>(".hud-error-layer");
const stopButton = document.querySelector<HTMLButtonElement>("#hud-stop");
const cancelButton = document.querySelector<HTMLButtonElement>("#hud-cancel");
const meetingStartButton = document.querySelector<HTMLButtonElement>("#hud-meeting-start");
const meetingAppLabel = document.querySelector<HTMLElement>("#hud-meeting-app");
const meetingVideoButton = document.querySelector<HTMLButtonElement>("#hud-meeting-video");
const meetingVideoLabel = document.querySelector<HTMLElement>("#hud-meeting-video-label");
let meetingVideoWindow: MeetingWindowDto | null = null;
let meetingVideoSelected = false;
let meetingPromptGeneration = 0;
let meetingVideoDetectionGeneration: number | undefined;

function renderMeetingVideo() {
  if (meetingVideoButton) {
    meetingVideoButton.hidden = !meetingVideoWindow;
    meetingVideoButton.setAttribute("aria-label", t("notes.editor.video.record"));
    meetingVideoButton.setAttribute("aria-checked", String(meetingVideoSelected));
    meetingVideoButton.title = t("notes.editor.video.permission");
  }
  if (meetingVideoLabel) meetingVideoLabel.textContent = t("notes.editor.video.record");
  if (hud) hud.dataset.meetingVideo = String(Boolean(meetingVideoWindow));
  if (meetingAppLabel && meetingVideoWindow) {
    meetingAppLabel.textContent = t("notes.editor.video.window", {
      app: meetingVideoWindow.appName,
      title: meetingVideoWindow.title,
    });
    meetingAppLabel.title = meetingAppLabel.textContent;
  } else {
    meetingAppLabel?.removeAttribute("title");
  }
}
meetingVideoButton?.addEventListener("click", (event) => {
  event.preventDefault();
  event.stopPropagation();
  if (hud?.dataset.state !== "meeting" || !meetingVideoWindow) return;
  meetingVideoSelected = !meetingVideoSelected;
  renderMeetingVideo();
});
const meetingDismissButton = document.querySelector<HTMLButtonElement>("#hud-meeting-dismiss");
const statusText = document.querySelector<HTMLElement>("#hud-status");
const escTip = document.querySelector<HTMLElement>("#hud-esc-tip");
const meetingLabel = document.querySelector<HTMLElement>("#hud-meeting-label");

// Replace a control's own text while keeping its child elements (the record
// button carries its microphone icon next to the label).
function setOwnText(element: HTMLElement | null, text: string) {
  if (!element) return;
  const node = Array.from(element.childNodes).find(
    (child) => child.nodeType === Node.TEXT_NODE && child.textContent?.trim(),
  );
  if (node) node.textContent = text;
  else element.append(document.createTextNode(text));
}

// The static page chrome from hud.html, in the current language.
function applyStaticLabels() {
  renderTargetLabel();
  renderMeetingVideo();
  document.title = t("hud.dictation.windowTitle");
  cancelButton?.setAttribute("aria-label", t("hud.dictation.cancel"));
  stopButton?.setAttribute("aria-label", t("hud.dictation.stop"));
  meetingDismissButton?.setAttribute("aria-label", t("hud.meeting.dismiss"));
  if (meetingLabel) meetingLabel.textContent = t("hud.meeting.detected");
  setOwnText(meetingStartButton, t("hud.meeting.record"));
}

applyStaticLabels();

// House iconography (central-icons), injected like the agent HUD does.
// The cancel X is the cross-medium cut at 16px: its drawn span (~8.3px)
// sits at optical parity with the stop's 8px solid square, while the muted
// resting tone keeps the escape hatch subordinate to the submit action. A
// bare X reads larger than its box — don't size it past the stop's mass.
if (cancelButton) {
  cancelButton.innerHTML = renderToStaticMarkup(
    createElement(IconCrossMedium, {
      size: 16,
      ariaHidden: true,
      focusable: false,
    }),
  );
}
// The Clovy dot spinner (the app-wide 3×3 mark), built by hand against the
// shared dot-spinner.css because this page has no React tree — same approach
// as the agent HUD's appendDotSpinner.
if (spinnerNode) {
  const spinner = document.createElement("span");
  spinner.className = "dot-spinner";
  spinner.dataset.size = "md";
  spinner.setAttribute("aria-hidden", "true");
  spinner.style.setProperty("--spinner-cols", String(SPINNER_GRID_COLS.md));
  for (const cell of spinnerGrid("md")) {
    const dot = document.createElement("span");
    dot.style.setProperty("--spinner-order", String(cell.order));
    if (cell.mark) {
      dot.dataset.mark = "";
    }
    spinner.appendChild(dot);
  }
  spinnerNode.appendChild(spinner);
}
if (meetingDismissButton) {
  meetingDismissButton.innerHTML = renderToStaticMarkup(
    createElement(IconCrossSmall, {
      size: 12,
      ariaHidden: true,
      focusable: false,
    }),
  );
}
if (errorIcon) {
  errorIcon.innerHTML = renderToStaticMarkup(
    createElement(IconExclamationCircle, {
      size: 16,
      ariaHidden: true,
      focusable: false,
    }),
  );
}
const meetingStartIcon = document.querySelector<HTMLElement>(".hud-meeting-start-icon");
if (meetingStartIcon) {
  meetingStartIcon.innerHTML = renderToStaticMarkup(
    createElement(IconMicrophone, {
      size: 14,
      ariaHidden: true,
      focusable: false,
    }),
  );
}

let hideTimer: number | undefined;
let meetingPromptTimer: number | undefined;
let longDictationNoticeTimer: number | undefined;
let meetingPromptSuppressed = false;
let pendingMeetingPrompt: DictationHudEvent | undefined;
let hideRequestId = 0;
let showRequestId = 0;
let showQueue: Promise<void> = Promise.resolve();

lifecycle.addCleanup(() => {
  // Invalidate async show/hide work that may still be awaiting a native
  // resize, font load, animation frame, or timeout when this webview dies.
  // Without this, an exit fallback from the old HUD instance can wake later
  // and hide a newly created window.
  hideRequestId += 1;
  showRequestId += 1;
  window.clearTimeout(hideTimer);
  window.clearTimeout(meetingPromptTimer);
  window.clearTimeout(longDictationNoticeTimer);
});

// Matches the .hud[data-state="exiting"] transition in hud.css. Long
// enough to read as a deliberate dissolve rather than a blink-out.
const EXIT_TRANSITION_MS = 240;
// Matches the .hud.is-morphing fade (60ms) plus a frame of slack.
const MORPH_FADE_MS = 80;
// Transparent margin around the frostless HUD surface. The compact HUD shadow
// paints here and the meeting dismiss can overhang the card's edge. Keep this
// in sync with --shadow-hud's maximum spread in tokens.css; using the larger
// app-card shadows here clips the Dictation HUD or forces a click-blocking window.
const WINDOW_GUTTER = 18;
// On macOS the Esc hint lives in a lightweight native click-through panel.
// Browser demos and other platforms keep the in-DOM fallback.
const HAS_TAURI_BRIDGE = "__TAURI_INTERNALS__" in window;
const USES_NATIVE_ESC_TOOLTIP =
  HAS_TAURI_BRIDGE && /Mac/i.test(`${navigator.platform} ${navigator.userAgent}`);
// The stop press collapses the wide pill down to the spinner square as a pure
// CSS width tween — the transparent native frame stays at the listening size
// (invisible) and snaps down only after the motion settles, so the frame can
// never clip the pill mid-flight. transitionend is authoritative; this only
// bounds the wait if an environment never dispatches it.
const COLLAPSE_SETTLE_FALLBACK_MS = 600;
const MEETING_PROMPT_TIMEOUT_MS = 30_000;
// A short dictation round-trips in well under a second. Past this, the
// user is watching a spinner wondering whether Clovy hung, so say so.
const LONG_DICTATION_NOTICE_MS = 6_000;

function invokeBestEffort(command: string, args?: Record<string, unknown>) {
  try {
    void Promise.resolve(invoke(command, args)).catch(() => {});
  } catch {
    // Native HUD commands are opportunistic; the visible state still advances.
  }
}

async function invokeBestEffortAsync(command: string, args?: Record<string, unknown>) {
  try {
    await Promise.resolve(invoke(command, args));
  } catch {
    // Native HUD commands are opportunistic; the visible state still advances.
  }
}

// Bar synthesis + ballistics live in the shared meter so the recorder waveform
// moves identically. The meter holds the level history and the displayed bars.
// Sized to the actual bar count so meter.displayed always matches bars.length.
const meter = createBarMeter(
  bars.length,
  HUD_BAR_WEIGHTS,
  HUD_BAR_HISTORY_OFFSETS,
  LIVE_WAVE_OPTIONS,
);

let rafHandle: number | undefined;
let shimmerTimer: number | undefined;

lifecycle.addCleanup(() => {
  window.clearTimeout(shimmerTimer);
});
let lastAudioLevelAt = 0;
const IDLE_RAF_TIMEOUT_MS = 260;
// Once the bars have settled and no fresh audio is arriving, the only thing
// left to animate is the slow idle carrier. Pace it at ~30fps via a timer
// instead of painting every rAF, so a long listening session doesn't pin the
// CPU at 60fps compositing — the carrier (a 0.45Hz sine) reads smooth either
// way. Full rAF resumes the moment audio or bar motion returns.
const SHIMMER_FRAME_MS = 33;
const AUDIO_NOISE_GATE = 0.02;
// Soft-knee speech curve, mirroring the recorder's scaleLiveInputPeak
// (LIVE_INPUT_KNEE / LIVE_INPUT_LOW_LIFT). The old sqrt(raw × 5) + clamp hit
// the ceiling at raw ≈ 0.21, so everything louder slammed flat at 1.0 and the
// bars jittered against the clamp. An exponential knee approaches 1.0
// asymptotically instead — loud speech compresses gracefully like the recorder
// does. The lift is a hair below the recorder's 0.6 so the HUD blooms a touch
// faster at conversational volume (close-mic dictation, one speaker).
// Lift note: a lower exponent makes the curve steeper near the bottom, which
// over-amplified the noisy quiet region — whispers flickered as tiny mic
// fluctuations swung the bars. 0.62 flattens the low end so quiet speech reads
// steady, at a small cost to how fast it blooms at conversational volume.
const AUDIO_KNEE = 6;
const AUDIO_LOW_LIFT = 0.62;
const AMBIENT_VISUAL_GAIN = 3;
const AMBIENT_MAX_LEVEL = 0.03;
const HUD_WHISPER_FLOOR = 0.06;
// Input peak-hold — the HUD's stand-in for the recorder's Rust-side
// max-over-window. The recorder reads max(recentPeaks) over ~66ms, which rides
// the upper envelope: it bridges the dips between syllables and stays steady on
// a noisy whisper instead of dropping into every trough. The HUD instead gets a
// stream of single level events, so we hold the peak and let it decay: each
// event jumps the held value up instantly to a louder sample (max) and bleeds
// it down by AUDIO_HOLD_DECAY otherwise. With this feeding the meter, the bars
// no longer bounce into inter-syllable gaps and the shared (recorder) ballistics
// can faithfully track the envelope. Lower decay = livelier/twitchier, higher =
// steadier but stickier. Slightly long so the tail eases down smoothly instead
// of dropping out from under the bars.
//
// The decay is time-normalized: the helper emits levels at ~50Hz (20ms) on the
// default-capture/recorder paths but ~25Hz (40ms) on the AVCapture path, so a
// flat per-event multiply would give a 2× different wall-clock hold depending
// on which metering source is live. AUDIO_HOLD_DECAY is the decay per
// HOLD_REF_EVENT_MS; each event scales it by elapsed dt so the hold lasts the
// same real time regardless of emit rate.
const AUDIO_HOLD_DECAY = 0.8;
const HOLD_REF_EVENT_MS = 20; // helper's nominal 50Hz emit cadence
let audioPeakHold = 0;
let lastHoldAt = 0; // performance.now() of the previous peak-hold update
// The idle pulse + speech wave live in the shared meter (IDLE_PULSE_*,
// SPEECH_WAVE_*, withWaveLayers) so the HUD and recorder move identically.

type HudTransition = {
  changed: boolean;
  id: number;
  previous?: string;
};

let hudTransitionId = 0;

// A status line is either a translatable label (resolved again when the
// interface language changes) or verbatim text such as a native error message.
type StatusText = string | (() => string);

const idleStatus = () => t("hud.status.idle");
const listeningStatus = () => t("hud.status.listening");
const transcribingStatus = () => t("hud.status.transcribing");
let currentStatus: StatusText = idleStatus;

function resolveStatus(status: StatusText) {
  return typeof status === "function" ? status() : status;
}

function setHud(state: string, statusSource: StatusText): HudTransition {
  const id = ++hudTransitionId;
  if (!hud || !statusText) return { changed: false, id };
  if (state === "idle" || state === "meeting") resetTarget();
  currentStatus = statusSource;
  const status = resolveStatus(statusSource);
  const previous = hud.dataset.state;
  const entersProcessing =
    (state === "transcribing" || state === "pasting") &&
    previous !== "transcribing" &&
    previous !== "pasting";
  hud.dataset.state = state;
  statusText.textContent = status;
  if (entersProcessing) {
    restartSpinnerAnimation();
  }
  // Keep the error message through "exiting" so it dissolves with the
  // window's native alpha fade instead of vanishing a frame early; the
  // post-hide flip to "idle" clears it.
  if (errorText && state !== "exiting") {
    errorText.textContent = state === "error" ? status : "";
  }
  // The error reveal collapse class is owned by the reveal/exit sequence.
  // Leave it alone for "error" and "exiting": the reveal removes it after
  // sizing, and error exits clear it before fading the expanded HUD in place.
  if (state !== "error" && state !== "exiting") {
    hud.classList.remove("hud-reveal-collapsed");
  }
  if (state !== "transcribing") {
    clearLongDictationNotice();
  }
  if (state === "listening") {
    startBarLoop();
    if (previous !== "listening") {
      pushStopBoundsToNative();
      pushCancelBoundsToNative();
    }
  } else if (previous === "listening") {
    cancelPendingAudioLevel();
    stopBarLoop();
    clearStopHover();
  }
  if (state !== previous && hud) {
    if (usesFrostlessChrome(previous) !== usesFrostlessChrome(state)) {
      // Kept for compatibility with older native windows; in normal builds
      // every state is frostless and this branch does not run.
      invokeBestEffort("dictation_hud_set_chrome", {
        frostless: usesFrostlessChrome(state),
      });
    }
    if (state === "meeting") clearStopHover();
  }
  return { changed: state !== previous, id, previous };
}

function restartSpinnerAnimation() {
  if (!spinnerNode || prefersReducedMotion()) return;
  // WebKit does not instantiate descendant CSS animations when their nearest
  // display:none ancestor first becomes visible. Briefly resolve the dots to
  // animation:none after the processing state makes the spinner displayable,
  // then restore the shared animation declaration from a clean rendered frame.
  spinnerNode.classList.add("hud-spinner-reset");
  spinnerNode.offsetWidth;
  spinnerNode.classList.remove("hud-spinner-reset");
}

async function updateErrorPlacement() {
  if (!hud || lifecycle.signal.aborted) return;
  let placement: unknown;
  try {
    placement = await Promise.resolve(invoke("dictation_hud_preferred_error_placement"));
  } catch {
    placement = undefined;
  }
  if (lifecycle.signal.aborted) return;
  hud.dataset.errorPlacement = placement === "above" ? "above" : "below";
}

function startBarLoop() {
  if (rafHandle !== undefined) return;
  // Audio arriving mid-shimmer cancels the throttled tick so we snap back to
  // full-rate rAF immediately instead of waiting out the timer.
  if (shimmerTimer !== undefined) {
    window.clearTimeout(shimmerTimer);
    shimmerTimer = undefined;
  }
  const tick = (now: number) => {
    rafHandle = undefined;
    const stillAnimating = meter.step();
    // Overall loudness = the tallest bar right now; drives the speech wave.
    let speech = 0;
    for (let i = 0; i < bars.length; i++) {
      speech = Math.max(speech, meter.displayed[i]);
    }
    for (let i = 0; i < bars.length; i++) {
      const level = withWaveLayers(meter.displayed[i], i, now, speech, bars.length);
      bars[i].style.setProperty("--level", level.toFixed(3));
    }
    const sinceAudio = performance.now() - lastAudioLevelAt;
    const reactive = stillAnimating || sinceAudio < IDLE_RAF_TIMEOUT_MS;
    // Once the idle pulse is on, keep animating for as long as we're listening
    // so the travelling pulse never freezes.
    const keepShimmering = IDLE_PULSE_AMP > 0 && hud?.dataset.state === "listening";
    if (reactive) {
      // Bars moving or audio recent → paint every frame for responsiveness.
      rafHandle = lifecycle.requestAnimationFrame(tick);
    } else if (keepShimmering) {
      // Idle but listening → throttle the carrier so the CPU can idle between ticks.
      shimmerTimer = window.setTimeout(() => {
        shimmerTimer = undefined;
        rafHandle = lifecycle.requestAnimationFrame(tick);
      }, SHIMMER_FRAME_MS);
    }
  };
  rafHandle = lifecycle.requestAnimationFrame(tick);
}

function stopBarLoop() {
  // A hidden WKWebView may suspend a queued animation frame indefinitely.
  // Clear both scheduling paths when listening ends so the next dictation
  // cannot mistake an orphaned callback for a live waveform loop.
  if (rafHandle !== undefined) {
    lifecycle.cancelAnimationFrame(rafHandle);
    rafHandle = undefined;
  }
  if (shimmerTimer !== undefined) {
    window.clearTimeout(shimmerTimer);
    shimmerTimer = undefined;
  }
}

function resetBars() {
  cancelPendingAudioLevel();
  meter.reset();
  audioPeakHold = 0;
  lastHoldAt = 0;
  for (let i = 0; i < bars.length; i++) {
    bars[i].style.setProperty("--level", IDLE_LEVEL.toFixed(3));
  }
  lastAudioLevelAt = performance.now();
}

function renderAudioLevel(rawLevel: number) {
  // Hold the raw envelope before shaping (mirrors the recorder windowing its
  // raw peaks before scaleLiveInputPeak): jump up to a louder sample instantly,
  // bleed down otherwise so the level rides through inter-syllable dips. The
  // decay is scaled by elapsed time so its wall-clock duration doesn't drift
  // with the helper's emit rate (see HOLD_REF_EVENT_MS). dt is clamped so a
  // first event or a post-idle gap doesn't collapse the hold in one step.
  const nowMs = performance.now();
  const dt = lastHoldAt ? clamp(nowMs - lastHoldAt, 1, 250) : HOLD_REF_EVENT_MS;
  lastHoldAt = nowMs;
  const decay = AUDIO_HOLD_DECAY ** (dt / HOLD_REF_EVENT_MS);
  audioPeakHold = Math.max(rawLevel, audioPeakHold * decay);
  const held = audioPeakHold;
  let shaped: number;
  if (held <= AUDIO_NOISE_GATE) {
    shaped = clamp(Math.sqrt(held * AMBIENT_VISUAL_GAIN), 0, AMBIENT_MAX_LEVEL);
  } else {
    // Continuous with the ambient branch: at gated → 0 the knee is 0, so
    // shaped starts at AMBIENT_MAX_LEVEL and rises toward 1.0 without clipping.
    const gated = (held - AUDIO_NOISE_GATE) / (1 - AUDIO_NOISE_GATE);
    const knee = 1 - Math.exp(-AUDIO_KNEE * gated ** AUDIO_LOW_LIFT);
    shaped = clamp(AMBIENT_MAX_LEVEL + (1 - AMBIENT_MAX_LEVEL) * knee, 0, 1);
  }
  if (held > AUDIO_NOISE_GATE && shaped > 0.0001 && HUD_WHISPER_FLOOR > 0) {
    shaped = HUD_WHISPER_FLOOR + (1 - HUD_WHISPER_FLOOR) * shaped;
  }
  lastAudioLevelAt = performance.now();
  meter.pushLevel(shaped);
  startBarLoop();
}

// Under CPU contention (e.g. a meeting recording competing for the machine),
// the helper's audio_level events arrive in bursts. The bar meter advances its
// travelling wave one history slot per pushLevel call (see audio-meter), so
// draining a burst within one frame lurches the wave several slots and the
// waveform jitters. Coalesce to at most one level per animation frame — the
// loudest pending sample — so the history advances at a steady frame rate no
// matter how bursty delivery is. Within a frame the peak-hold takes the max
// anyway, so this preserves the shaping while smoothing the motion.
let pendingRawLevel: number | null = null;
let levelFlushHandle: number | undefined;

function queueAudioLevel(rawLevel: number) {
  pendingRawLevel = pendingRawLevel === null ? rawLevel : Math.max(pendingRawLevel, rawLevel);
  if (levelFlushHandle !== undefined) return;
  levelFlushHandle = lifecycle.requestAnimationFrame(() => {
    levelFlushHandle = undefined;
    const next = pendingRawLevel;
    pendingRawLevel = null;
    if (next !== null) renderAudioLevel(next);
  });
}

function cancelPendingAudioLevel() {
  if (levelFlushHandle !== undefined) {
    lifecycle.cancelAnimationFrame(levelFlushHandle);
    levelFlushHandle = undefined;
  }
  pendingRawLevel = null;
}

function setStopHover(isHovered: boolean) {
  stopButton?.classList.toggle("is-hovered", isHovered);
}

function setCancelHover(isHovered: boolean) {
  cancelButton?.classList.toggle("is-hovered", isHovered);
}

// Standalone browser demos own their key events. Native helpers must
// explicitly advertise a global Escape monitor before the HUD teaches it.
let escapeCancelAvailable = !HAS_TAURI_BRIDGE;
let activeDictationTakeId: string | undefined;
// Take IDs are never reused. Keep terminal identities for this webview's
// lifetime so queued outcomes cannot revive a cancelled/completed indicator.
const retiredDictationTakeIds = new Set<string>();
function retireDictationTake() {
  if (activeDictationTakeId) retiredDictationTakeIds.add(activeDictationTakeId);
  activeDictationTakeId = undefined;
  cleanupSkippedReason = undefined;
}
/** Cleanup failed for the take being pasted: its raw transcript is pasted,
 * and the paste status says so instead of raising an error. */
let cleanupSkippedReason: "openrouter_api_key_missing" | "other" | undefined;
let receivedLiveDictationEvent = false;

function dictationTakeId(event: DictationHudEvent) {
  const takeId = event.payload?.takeId;
  return typeof takeId === "string" && takeId.trim().length > 0 && takeId.length <= 128
    ? takeId.trim()
    : undefined;
}

function correlatedHelperCommand(type: "stop_and_paste" | "discard_recording") {
  return activeDictationTakeId ? { type, takeId: activeDictationTakeId } : { type };
}

function setCancelTooltipHover(isHovered: boolean) {
  setEscTipVisible(isHovered && escapeCancelAvailable && hud?.dataset.state === "listening");
}

// The Esc hint is a real tooltip beside the pill. macOS renders it in a
// separate click-through native panel; the standalone/browser fallback keeps
// its slot reserved in the HUD frame. Neither path resizes the visible pill
// on hover, which would stale the control rects polled by the native thread.
let escTipOpen = false;
let escTipToken = 0;

lifecycle.addCleanup(() => {
  escTipOpen = false;
  escTipToken += 1;
  hud?.classList.remove("hud-esc-tip-open");
  if (USES_NATIVE_ESC_TOOLTIP) {
    invokeBestEffort("dictation_hud_set_tooltip_visible", { visible: false });
  }
});

function setEscTipVisible(visible: boolean) {
  if (!hud || escTipOpen === visible) return;
  escTipOpen = visible;
  const token = ++escTipToken;
  if (!visible) {
    hud.classList.remove("hud-esc-tip-open");
    if (USES_NATIVE_ESC_TOOLTIP) {
      invokeBestEffort("dictation_hud_set_tooltip_visible", { visible: false });
    }
    return;
  }
  void updateEscTipPlacement().then(() => {
    if (lifecycle.signal.aborted || token !== escTipToken || !escTipOpen) return;
    if (USES_NATIVE_ESC_TOOLTIP) {
      invokeBestEffort("dictation_hud_set_tooltip_visible", {
        visible: true,
        placement: hud?.dataset.tipPlacement,
      });
    } else {
      hud?.classList.add("hud-esc-tip-open");
    }
  });
}

// Same screen-half heuristic the error layer uses: the tip flips above the
// pill when the pill sits in the lower half of the screen.
async function updateEscTipPlacement() {
  if (!hud || lifecycle.signal.aborted) return;
  let placement: unknown;
  try {
    placement = await Promise.resolve(invoke("dictation_hud_preferred_error_placement"));
  } catch {
    placement = undefined;
  }
  if (lifecycle.signal.aborted) return;
  hud.dataset.tipPlacement = placement === "above" ? "above" : "below";
}

function setDismissHover(isHovered: boolean) {
  meetingDismissButton?.classList.toggle("is-hovered", isHovered);
}

// Meeting-card hover, computed natively against rects pushed from here —
// CSS :hover is unreliable on the non-key HUD panel, see .hud-stop.
// Hovering anywhere over the card (plus the overhanging X) reveals the
// corner dismiss; hovering the record button paints its hover wash.
function pushDismissBoundsToNative() {
  if (hud?.dataset.state !== "meeting") {
    invokeBestEffort("dictation_hud_set_dismiss_bounds", { rect: null });
    invokeBestEffort("dictation_hud_set_record_bounds", { rect: null });
    return;
  }
  const card = hud.getBoundingClientRect();
  const cross = meetingDismissButton?.getBoundingClientRect();
  invokeBestEffort("dictation_hud_set_dismiss_bounds", {
    rect: {
      left: Math.min(card.left, cross?.left ?? card.left),
      top: Math.min(card.top, cross?.top ?? card.top),
      right: card.right,
      bottom: card.bottom,
    },
  });
  const record = meetingStartButton?.getBoundingClientRect();
  invokeBestEffort("dictation_hud_set_record_bounds", {
    rect: record
      ? {
          left: record.left,
          right: record.right,
          top: record.top,
          bottom: record.bottom,
        }
      : null,
  });
}

function setRecordHover(isHovered: boolean) {
  meetingStartButton?.classList.toggle("is-hovered", isHovered);
}

function clearDismissHover() {
  setDismissHover(false);
  setRecordHover(false);
  invokeBestEffort("dictation_hud_set_dismiss_bounds", { rect: null });
  invokeBestEffort("dictation_hud_set_record_bounds", { rect: null });
}

// Hover + click pass-through are computed in Rust against rects we push from
// here. WebKit throttles JS timers on the non-key HUD panel, so any polling
// done in JS only fires reliably during a mouse-down.
function pushStopBoundsToNative() {
  if (!stopButton || hud?.dataset.state !== "listening") {
    invokeBestEffort("dictation_hud_set_stop_bounds", { rect: null });
    return;
  }
  const { left, right, top, bottom } = stopButton.getBoundingClientRect();
  invokeBestEffort("dictation_hud_set_stop_bounds", {
    rect: { left, right, top, bottom },
  });
}

function pushCancelBoundsToNative() {
  if (!cancelButton || hud?.dataset.state !== "listening") {
    invokeBestEffort("dictation_hud_set_cancel_bounds", { rect: null });
    return;
  }
  const { left, right, top, bottom } = cancelButton.getBoundingClientRect();
  invokeBestEffort("dictation_hud_set_cancel_bounds", {
    rect: { left, right, top, bottom },
  });
}

// The native window size the current state wants. getBoundingClientRect
// includes transforms, and the exit leaves one in play: "exiting" eases the
// pill to scale(0.94), and when the window hides mid-flight the suspended
// webview freezes that transition wherever it stood until the next show. A
// fresh show measured through the frozen scale baked a too-narrow frame
// into the window — the pill came up clipped flat at the right edge, then
// visibly snapped to full width when the settle pass healed it. Kill
// in-flight transitions for the measurement so every property sits at its
// end value.
function measureWindowSize() {
  if (!hud) return { width: 0, height: 0 };
  hud.classList.add("hud-snap");
  hud.offsetWidth;
  let { width, height } = hud.getBoundingClientRect();
  if (hud.dataset.state === "error" && errorLayer) {
    // The message layer is centered above/below the pill (.hud-snap forces
    // it to its full drawn-out height for this measurement). Mirror its slot
    // on the opposite vertical side so the center-anchored native resize
    // leaves the pill dead centre while the layer reveals vertically.
    const layer = errorLayer.getBoundingClientRect();
    width = Math.max(width, layer.width);
    height += 2 * (cssPixelToken("--sp-2") + layer.height);
  }
  if (
    hud.dataset.state === "listening" &&
    escTip &&
    escapeCancelAvailable &&
    !USES_NATIVE_ESC_TOOLTIP
  ) {
    // The browser/non-macOS fallback reserves the Esc tooltip slot for the
    // whole session, mirrored vertically so the pill sits dead-center.
    // macOS uses a separate click-through native panel and skips this band.
    const tip = escTip.getBoundingClientRect();
    width = Math.max(width, tip.width);
    height += 2 * (cssPixelToken("--sp-2") + tip.height);
  }
  if (usesFrostlessChrome(hud.dataset.state)) {
    // The frostless HUD includes the transparent gutter its CSS shadow (and
    // the meeting card's overhanging dismiss) paint into. The center-anchored
    // native resize splits it evenly, so the surface stays put.
    width += WINDOW_GUTTER * 2;
    height += WINDOW_GUTTER * 2;
  }
  hud.classList.remove("hud-snap");
  return { width, height };
}

// Resize the native window to the pill's measured size (Rust re-anchors so
// the pill center stays put), then refresh the stop-button rect once layout
// has settled at the new size — the pill's client position shifts when the
// window around it changes. With `morph` the contents fade out while the
// glass eases to its new frame, then fade back in (the invoke resolves when
// the native motion finishes).
async function syncWindowToPill(options?: { animate?: boolean; morph?: boolean }) {
  if (!hud || lifecycle.signal.aborted) return;
  // A stop-collapse is pure CSS under a stationary frame; let it settle
  // before measuring, or the frame would snap down mid-motion and clip it.
  while (collapseSettled) {
    await collapseSettled;
    if (lifecycle.signal.aborted) return;
  }
  // ABC Diatype may still be loading on the window's first show; measuring
  // with the fallback font bakes the wrong width into the window frame.
  if (typeof document.fonts?.ready?.then === "function") {
    if (document.fonts.status === "loading") {
      try {
        await document.fonts.ready;
      } catch {
        // Best effort; the fallback metrics are close enough to recover on
        // the next state change.
      }
    }
  }
  if (lifecycle.signal.aborted) return;
  if (options?.morph) {
    hud.classList.add("is-morphing");
    // Let the contents finish fading before the glass starts moving: the
    // webview lays out at the final size immediately, so anything still
    // visible during the native ease paints clipped by the old frame —
    // worst on the meeting card, which changes height as well as width.
    if (!prefersReducedMotion()) {
      await new Promise((resolve) => window.setTimeout(resolve, MORPH_FADE_MS));
      if (lifecycle.signal.aborted) return;
    }
  }
  const { width, height } = measureWindowSize();
  // Exact floats — Rust rounds at physical pixels. Ceiling here oversized
  // the window by up to a point, leaving a bright sliver of bare frost
  // around the dark card.
  await invokeBestEffortAsync("dictation_hud_set_size", {
    width,
    height,
    animate: options?.animate ?? !prefersReducedMotion(),
  });
  if (lifecycle.signal.aborted) return;
  lifecycle.requestAnimationFrame(() => {
    lifecycle.requestAnimationFrame(() => {
      hud?.classList.remove("is-morphing");
      pushStopBoundsToNative();
      pushCancelBoundsToNative();
      pushDismissBoundsToNative();
    });
  });
}

// The native window alpha drives the exit dissolve so the CSS surface, shadow,
// and transparent gutter fade as one unit.
function setWindowAlpha(alpha: number) {
  invokeBestEffort("dictation_hud_set_alpha", { alpha });
}

function fadeWindowAlpha(requestId: number) {
  return new Promise<void>((resolve) => {
    const start = performance.now();
    const step = (now: number) => {
      if (requestId !== hideRequestId) {
        resolve();
        return;
      }
      const t = Math.min((now - start) / EXIT_TRANSITION_MS, 1);
      setWindowAlpha(1 - t);
      if (t < 1) {
        lifecycle.requestAnimationFrame(step);
      } else {
        resolve();
      }
    };
    lifecycle.requestAnimationFrame(step);
  });
}

// Fallback entrance for the standalone browser page, where the native
// slide+fade (dictation_hud_show with enter) has no bridge to run on.
function replayCssEntrance() {
  if (!hud) return;
  hud.classList.remove("hud-enter");
  hud.offsetWidth;
  hud.classList.add("hud-enter");
}

// matchMedia is absent in the jsdom test environment.
function prefersReducedMotion() {
  return (
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

// The macOS "not allowed" wobble, done natively — the window jiggles, the
// frost moves with it (a CSS translateX would slide the tint off the
// stationary vibrancy view).
function triggerShake() {
  if (prefersReducedMotion()) return;
  invokeBestEffort("dictation_hud_shake");
}

function clearStopHover() {
  setStopHover(false);
  setCancelHover(false);
  setCancelTooltipHover(false);
  invokeBestEffort("dictation_hud_set_stop_bounds", { rect: null });
  invokeBestEffort("dictation_hud_set_cancel_bounds", { rect: null });
}

function clearHideTimer() {
  if (hideTimer) {
    window.clearTimeout(hideTimer);
    hideTimer = undefined;
  }
}

function clearMeetingPromptTimer() {
  if (meetingPromptTimer) {
    window.clearTimeout(meetingPromptTimer);
    meetingPromptTimer = undefined;
  }
}

function startMeetingPromptTimer() {
  if (lifecycle.signal.aborted || meetingPromptTimer !== undefined) return;
  meetingPromptTimer = window.setTimeout(() => {
    meetingPromptTimer = undefined;
    if (lifecycle.signal.aborted) return;
    if (hud?.dataset.state !== "meeting") return;
    meetingPromptSuppressed = true;
    void hideHud();
  }, MEETING_PROMPT_TIMEOUT_MS);
}

function clearLongDictationNotice() {
  if (longDictationNoticeTimer !== undefined) {
    window.clearTimeout(longDictationNoticeTimer);
    longDictationNoticeTimer = undefined;
  }
}

function startLongDictationNotice() {
  if (lifecycle.signal.aborted || longDictationNoticeTimer !== undefined) return;
  longDictationNoticeTimer = window.setTimeout(() => {
    longDictationNoticeTimer = undefined;
    if (lifecycle.signal.aborted) return;
    if (hud?.dataset.state !== "transcribing") return;
    const transition = setHud("transcribing", () => t("hud.status.stillTranscribing"));
    void showHud(showOptionsForTransition(transition));
  }, LONG_DICTATION_NOTICE_MS);
}

async function hideHud() {
  if (lifecycle.signal.aborted) return;
  retireDictationTake();
  targetActive = false;
  targetGeneration += 1;
  const requestId = ++hideRequestId;
  showRequestId += 1;
  clearHideTimer();
  clearFrameSettleTimer();
  clearMeetingPromptTimer();
  clearStopHover();
  clearDismissHover();
  const exitState = hud?.dataset.state;
  const exitingError = exitState === "error";
  if (exitingError) {
    hud?.classList.remove("hud-reveal-collapsed");
  }
  let nativeExit = false;
  if (hud) {
    const meetingExit = hud.dataset.state === "meeting" && !prefersReducedMotion();
    hud.classList.toggle("hud-exit-up", meetingExit);
    hud.classList.toggle("hud-error-exit", exitingError);
    if (exitState) {
      hud.dataset.exitState = exitState;
    }
    setHud("exiting", currentStatus);
    if (meetingExit) {
      // The meeting card leaves the way it came in: a native slide-up +
      // fade that also hides the window (the invoke resolves once it's
      // hidden). CSS can't do the motion — see showHud.
      try {
        await invoke("dictation_hud_exit");
        nativeExit = true;
      } catch {
        // No bridge: fall through to the plain alpha fade.
      }
    }
    if (!nativeExit) {
      // CSS dissolves the content; the native alpha ramp fades the whole
      // transparent window with it. The timeout race guards against rAF
      // stalling if the window is already occluded/hidden.
      await Promise.race([
        fadeWindowAlpha(requestId),
        new Promise((resolve) => window.setTimeout(resolve, EXIT_TRANSITION_MS + 60)),
      ]);
    }
  }
  if (requestId !== hideRequestId) return;
  // hide() rejects on the standalone browser page (no Tauri bridge); the
  // demo driver still needs the state machine to advance.
  await appWindow?.hide().catch(() => {});
  if (lifecycle.signal.aborted || requestId !== hideRequestId) return;
  setWindowAlpha(1);
  // Don't park on "exiting" (opacity 0, pointer-events none): if the native
  // window is ever shown again without new content, a pill stuck in that
  // state renders as a bare, undraggable gray bar.
  if (hud?.dataset.state === "exiting" && requestId === hideRequestId) {
    hud.classList.remove("hud-error-exit");
    delete hud.dataset.exitState;
    setHud("idle", idleStatus);
  }
}

function shouldMorphTransition(transition: HudTransition) {
  return (
    transition.changed &&
    transition.previous !== undefined &&
    !pillIsBlank(transition.previous) &&
    !pillIsBlank(hud?.dataset.state)
  );
}

function showOptionsForTransition(transition: HudTransition) {
  const current = hud?.dataset.state;
  // The processing chain never morphs: transcribing and pasting share the
  // compact spinner layout (a fade would blink the spinner), and leaving
  // "listening" is handled by collapseToSpinner's CSS width tween — this
  // path only sees it as a snap fallback (reduced motion, zero-width pill).
  const processingContinuation =
    (transition.previous === "listening" && current === "transcribing") ||
    (transition.previous === "transcribing" && current === "pasting") ||
    (transition.previous === "pasting" && current === "pasting");
  const morph = !processingContinuation && shouldMorphTransition(transition);
  return {
    fresh: transition.changed && pillIsBlank(transition.previous),
    morph,
  };
}

function showHud(options?: { fresh?: boolean; morph?: boolean }) {
  if (lifecycle.signal.aborted) return Promise.resolve();
  const requestId = ++showRequestId;
  hideRequestId += 1;
  clearHideTimer();
  clearFrameSettleTimer();

  const run = showQueue.then(() => showHudNow(requestId, options));
  showQueue = run.catch(() => {});
  return run;
}

async function showHudNow(requestId: number, options?: { fresh?: boolean; morph?: boolean }) {
  if (lifecycle.signal.aborted || requestId !== showRequestId) return;
  if (options?.fresh) {
    setWindowAlpha(0);
  }
  // Size the window to the pill before it appears, then let Rust position
  // and show it (dictation_hud_show also restores the native alpha an
  // interrupted exit fade may have left low). Showing only after the resize
  // is what keeps the pill from flashing up as a bare gray bar, or clipped
  // at a stale width from a previous state.
  await syncWindowToPill({
    animate: options?.morph ? !prefersReducedMotion() : false,
    morph: options?.morph,
  });
  if (lifecycle.signal.aborted || requestId !== showRequestId) return;
  // A fresh meeting prompt always enters at the top-center default spot,
  // and (motion permitting) slides down from the top edge while the
  // window alpha ramps up. The motion is native (the invoke resolves when
  // it settles): a CSS translate would slide the card off the stationary
  // window chrome, flashing bare edges.
  const meetingEntrance = hud?.dataset.state === "meeting";
  const animate = !prefersReducedMotion();
  try {
    await invoke("dictation_hud_show", {
      enter: meetingEntrance ? true : null,
      animate,
    });
  } catch {
    // No bridge (standalone page): fall back to the CSS entrance.
    if (meetingEntrance && animate) replayCssEntrance();
  }
  if (lifecycle.signal.aborted || requestId !== showRequestId) return;
  // Force a layout flush before reading rects.
  hud?.offsetWidth;
  if (hud?.dataset.state === "meeting") {
    clearStopHover();
    pushDismissBoundsToNative();
  } else {
    pushStopBoundsToNative();
    pushCancelBoundsToNative();
    pushDismissBoundsToNative();
  }
  assertWindowMatchesPill();
}

// One settle pass after every show. A long-hidden webview can wake with a
// stale frame: the show lands before its resize, parking the pill clipped
// with bare frost poking out — and nothing later heals it, because parked
// states (a demo-driven "listening" stream of audio levels, say) never
// trigger another resize. The webview viewport tracks the native content
// size, so a viewport/pill mismatch after layout settles means the frame
// race was lost; re-issue the resize.
const FRAME_SETTLE_DELAY_MS = 120;
let frameSettleTimer: number | undefined;

lifecycle.addCleanup(() => {
  window.clearTimeout(frameSettleTimer);
});

function clearFrameSettleTimer() {
  if (frameSettleTimer !== undefined) {
    window.clearTimeout(frameSettleTimer);
    frameSettleTimer = undefined;
  }
}

function assertWindowMatchesPill() {
  // Standalone browser page: the viewport is the whole tab, never the pill.
  if (!appWindow || lifecycle.signal.aborted) return;
  clearFrameSettleTimer();
  frameSettleTimer = window.setTimeout(() => {
    frameSettleTimer = undefined;
    if (lifecycle.signal.aborted) return;
    const state = hud?.dataset.state;
    if (!hud || !state || state === "idle" || state === "exiting") return;
    // Never measure mid-collapse: hud-snap would jump the width tween to its
    // end value. The collapse runs its own snap resize when it settles.
    if (collapseSettled) return;
    const expected = measureWindowSize();
    const drift = Math.max(
      Math.abs(window.innerWidth - expected.width),
      Math.abs(window.innerHeight - expected.height),
    );
    // Rust rounds the frame at physical pixels; allow a point of slack.
    // Snap, don't ease: this corrects a pill the user already sees as
    // settled, and easing it reads as the window morphing on its own.
    if (drift > 1.5) void syncWindowToPill({ animate: false });
  }, FRAME_SETTLE_DELAY_MS);
}

function hideSoon(delay = 900) {
  if (lifecycle.signal.aborted) return;
  clearHideTimer();
  hideTimer = window.setTimeout(() => {
    if (lifecycle.signal.aborted) return;
    void hideHud();
  }, delay);
}

async function handleDictationEventPayload(payload: unknown) {
  const dictationEvent = parseEvent(payload);
  if (!dictationEvent) return;
  if (activeDictationTakeId && dictationEvent.payload?.preserveActiveTake === true) {
    return;
  }
  const eventTakeId = dictationTakeId(dictationEvent);
  if (eventTakeId && retiredDictationTakeIds.has(eventTakeId)) return;
  if (
    activeDictationTakeId &&
    eventTakeId &&
    eventTakeId !== activeDictationTakeId &&
    [
      "audio_level",
      "focus_target",
      "finalizing_transcript",
      "final_transcript",
      "dictation_cleanup_skipped",
      "paste_target",
      "paste_completed",
      "recording_discarded",
      "error",
    ].includes(dictationEvent.type)
  ) {
    return;
  }

  if (dictationEvent.type === "dictation_cleanup_skipped") {
    cleanupSkippedReason =
      dictationEvent.payload?.code === "openrouter_api_key_missing"
        ? "openrouter_api_key_missing"
        : "other";
    return;
  }

  if (dictationEvent.type === "listening_started") {
    if (eventTakeId !== activeDictationTakeId) retireDictationTake();
    cleanupSkippedReason = undefined;
    resetTarget();
    targetActive = true;
    if (targetNode) targetNode.hidden = false;
    updateTarget(dictationEvent.payload?.target);
    activeDictationTakeId = dictationTakeId(dictationEvent);
    escapeCancelAvailable =
      !HAS_TAURI_BRIDGE || dictationEvent.payload?.escapeCancelAvailable === true;
    resetBars();
    const transition = setHud("listening", listeningStatus);
    await showHud(showOptionsForTransition(transition));
    return;
  }

  if (dictationEvent.type === "focus_target") {
    // Candidate updates belong only to listening. A late subscriber may be
    // pasting before it receives authoritative destination metadata.
    if (
      !targetActive ||
      targetPinned ||
      hud?.dataset.state !== "listening" ||
      (activeDictationTakeId && !eventTakeId)
    ) {
      return;
    }
    updateTarget(dictationEvent.payload?.target);
    return;
  }

  if (dictationEvent.type === "escape_cancel_unavailable") {
    escapeCancelAvailable = false;
    setEscTipVisible(false);
    return;
  }

  if (dictationEvent.type === "audio_level") {
    // The helper flushes a final coalesced level when the recorder stops, which
    // arrives AFTER finalizing_transcript. Once we've moved past listening, that
    // stray level must NOT pull the HUD back to "listening" — otherwise it kills
    // the processing spinner and the pill looks stuck until the paste lands.
    const state = hud?.dataset.state;
    if (
      state === "idle" ||
      state === "transcribing" ||
      state === "pasting" ||
      state === "error" ||
      state === "exiting"
    ) {
      return;
    }
    const level = Number(dictationEvent.payload?.level || 0);
    queueAudioLevel(level);
    setHud("listening", listeningStatus);
    return;
  }

  if (dictationEvent.type === "finalizing_transcript") {
    targetActive = true;
    if (Object.hasOwn(dictationEvent.payload ?? {}, "target")) {
      updateTarget(dictationEvent.payload?.target);
    }
    targetPinned = true;
    if (targetNode) targetNode.hidden = false;
    // Drop any level still queued from listening so it can't push a stray
    // sample into the meter after we've moved on to transcribing.
    cancelPendingAudioLevel();
    if (hud?.dataset.state === "listening") {
      // Push-to-talk release (or any stop that skipped the button): same
      // collapse the stop button plays.
      await collapseToSpinner(transcribingStatus);
    } else {
      const transition = setHud("transcribing", transcribingStatus);
      await showHud(showOptionsForTransition(transition));
    }
    if (lifecycle.signal.aborted) return;
    startLongDictationNotice();
    return;
  }

  if (dictationEvent.type === "final_transcript") {
    targetActive = true;
    if (targetNode) targetNode.hidden = false;
    if (!targetPinned && Object.hasOwn(dictationEvent.payload ?? {}, "target")) {
      updateTarget(dictationEvent.payload?.target);
      targetPinned = true;
    }
    const transition = setHud("pasting", () => t("hud.status.pasting"));
    await showHud(showOptionsForTransition(transition));
    return;
  }

  if (dictationEvent.type === "paste_target") {
    targetActive = true;
    if (targetNode) targetNode.hidden = false;
    if (!targetPinned && Object.hasOwn(dictationEvent.payload ?? {}, "target")) {
      updateTarget(dictationEvent.payload?.target);
    }
    targetPinned = true;
    const app = dictationEvent.payload?.app;
    const statusKey =
      cleanupSkippedReason === "openrouter_api_key_missing"
        ? "hud.status.pastingIntoMissingOpenRouterKey"
        : cleanupSkippedReason
          ? "hud.status.pastingIntoUncleaned"
          : "hud.status.pastingInto";
    const transition = setHud("pasting", () =>
      t(statusKey, {
        app: app || t("hud.status.previousApp"),
      }),
    );
    await showHud(showOptionsForTransition(transition));
    return;
  }

  if (dictationEvent.type === "helper_unavailable" || dictationEvent.type === "shutdown_ack") {
    retireDictationTake();
    void hideHud();
    return;
  }

  if (dictationEvent.type === "paste_completed") {
    const showMissingKeyNotice = cleanupSkippedReason === "openrouter_api_key_missing";
    retireDictationTake();
    if (showMissingKeyNotice) {
      hideSoon(3000);
    } else {
      void hideHud();
    }
    return;
  }

  if (dictationEvent.type === "recording_discarded") {
    retireDictationTake();
    // A grazed push-to-talk key or a signed-out session: the recording was
    // dropped without transcription, so the listening HUD just goes away.
    void hideHud();
    return;
  }

  if (dictationEvent.type === "agent_session_prompt") {
    void hideHud();
    return;
  }

  if (dictationEvent.type === "error") {
    // Rust pre-classifies via payload.silent so the HUD has one source of
    // truth for what counts as a "Nothing recorded" case. When nothing was
    // recorded there's nothing to tell the user: take the normal graceful
    // exit (the same fade hideHud runs after a successful paste) and say
    // nothing. Real failures fall through to the visible error treatment
    // below: Rust's promotion logic upgrades a silent classification to a
    // real error when speech was probably detected, so those still show.
    if (dictationEvent.payload?.silent === true) {
      void hideHud();
      return;
    }
    // Stop with nothing running (a stop racing a session that already
    // ended, or the demo pill's stop button hitting the real helper): the
    // desired end state — not listening — is already true, so there is
    // nothing to tell the user. If key-down already produced a useful error
    // (for example, missing microphone permission), preserve that error until
    // its normal timeout instead of letting the secondary key-up failure hide
    // it immediately.
    if (dictationEvent.payload?.code === "not_listening") {
      if (hud?.dataset.state === "error") return;
      void hideHud();
      return;
    }
    // Re-triggering dictation while the pill is already up listening: the
    // wobble on the live pill says "already going" without a text toast —
    // which would also replace the listening state and auto-hide while the
    // recording is still running. Listening continues untouched (the helper
    // keeps streaming audio levels, which also keeps Rust's post-error
    // window-hide timer from firing).
    if (
      dictationEvent.payload?.code === "already_listening" &&
      hud?.dataset.state === "listening"
    ) {
      triggerShake();
      return;
    }
    targetPinned = true;
    retireDictationTake();
    // Native errors carry a stable code; the interface language words it, and
    // the native (English) message is only the fallback for unknown codes.
    const message = messageForErrorCode(
      dictationEvent.payload?.code,
      String(dictationEvent.payload?.message ?? "").trim(),
    );
    const transition = setHud("error", message || (() => t("hud.status.dictationFailed")));
    // Latch the error state before awaiting native placement. A key-up event
    // can arrive during that IPC call, and its secondary not_listening error
    // must not dismiss the actionable start failure.
    await updateErrorPlacement();
    if (lifecycle.signal.aborted || transition.id !== hudTransitionId) return;
    // Render the pill with the message layer drawn in, snap the window to
    // the full (layer-included) size with no native motion, then draw the
    // layer out in CSS — the pill never moves and nothing clips.
    hud?.classList.add("hud-reveal-collapsed");
    await showHud({ fresh: pillIsBlank(transition.previous) });
    if (lifecycle.signal.aborted || transition.id !== hudTransitionId) return;
    playErrorReveal();
    triggerShake();
    // Hold long enough for the shake to finish and the message to read.
    hideSoon(1800);
  }
}

// Drop the collapse so the message layer draws out of the pill. The pill was
// rendered collapsed and the window already snapped to the full size, so
// this is a pure CSS draw-out within the still window.
function playErrorReveal() {
  if (!hud) return;
  if (prefersReducedMotion()) {
    hud.classList.remove("hud-reveal-collapsed");
    return;
  }
  lifecycle.requestAnimationFrame(() =>
    lifecycle.requestAnimationFrame(() => hud?.classList.remove("hud-reveal-collapsed")),
  );
}

async function handleMeetingDetectionEventPayload(payload: unknown) {
  const meetingEvent = parseEvent(payload);
  if (!meetingEvent) return;

  if (meetingEvent.type === "meeting_detected") {
    if (!isOnboardingComplete()) {
      pendingMeetingPrompt = meetingEvent;
      hideBlankWindowIfNeeded();
      return;
    }
    pendingMeetingPrompt = undefined;
    await showMeetingPrompt(meetingEvent);
    return;
  }

  if (meetingEvent.type === "meeting_cleared") {
    pendingMeetingPrompt = undefined;
    meetingPromptSuppressed = false;
    clearMeetingPromptTimer();
    if (hud?.dataset.state === "meeting") {
      void hideHud();
    } else if (pillIsBlank(hud?.dataset.state)) {
      // Heal a contentless window left visible by an earlier show.
      void appWindow?.hide().catch(() => {});
    }
  }
}

async function showMeetingPrompt(meetingEvent: DictationHudEvent) {
  if (lifecycle.signal.aborted) return;
  if (meetingPromptSuppressed || !canShowMeetingPrompt(hud?.dataset.state)) {
    // Rust may have shown the native window before emitting this event.
    // When the prompt won't render and the pill has no other content, put
    // the window back down — otherwise only the frosted surface shows: a
    // gray bar that can't be dragged or dismissed.
    hideBlankWindowIfNeeded();
    return;
  }
  const opening = hud?.dataset.state !== "meeting";
  const generation = opening ? ++meetingPromptGeneration : meetingPromptGeneration;
  if (opening) {
    meetingVideoSelected = false;
    meetingVideoWindow = null;
  }
  let detection: Promise<MeetingWindowDto | null> | undefined;
  if (
    !meetingVideoWindow &&
    meetingVideoDetectionGeneration !== generation &&
    isMacLikePlatform()
  ) {
    meetingVideoDetectionGeneration = generation;
    detection = detectMeetingWindow().catch(() => null);
  }
  // Set the app line before the pill is measured so the window is sized
  // for it. Heartbeats refresh it (the mic can move between apps).
  meetingAppLabels = meetingEvent.payload?.appLabels;
  if (meetingAppLabel) {
    meetingAppLabel.textContent = meetingAppLine(meetingAppLabels);
  }
  renderMeetingVideo();
  const transition = setHud("meeting", () => t("hud.meeting.detected"));
  try {
    await showHud(showOptionsForTransition(transition));
    if (lifecycle.signal.aborted || generation !== meetingPromptGeneration) return;
    startMeetingPromptTimer();
    if (detection) {
      const window = await detection;
      if (
        lifecycle.signal.aborted ||
        generation !== meetingPromptGeneration ||
        hud?.dataset.state !== "meeting" ||
        meetingPromptSuppressed ||
        meetingVideoWindow
      )
        return;
      meetingVideoWindow = window;
      renderMeetingVideo();
      if (window) await syncWindowToPill({ animate: false });
    }
  } finally {
    // Hold the slot through rendering. An old prompt must not release a
    // lookup belonging to its replacement, nor can heartbeats overlap retries.
    if (detection && meetingVideoDetectionGeneration === generation) {
      meetingVideoDetectionGeneration = undefined;
    }
  }
}

let meetingAppLabels: unknown;

function hideBlankWindowIfNeeded() {
  if (pillIsBlank(hud?.dataset.state)) {
    void appWindow?.hide().catch(() => {});
  }
}

function showPendingMeetingPromptAfterOnboarding() {
  if (lifecycle.signal.aborted || !pendingMeetingPrompt || !isOnboardingComplete()) return;
  const meetingEvent = pendingMeetingPrompt;
  pendingMeetingPrompt = undefined;
  void showMeetingPrompt(meetingEvent);
}

// "Zoom" / "Zoom, Chrome" — the friendly labels Rust derives from the
// processes holding the microphone. Detection is mic-based, so when no
// label survives validation, say what we actually know.
function meetingAppLine(labels: unknown) {
  const names = Array.isArray(labels)
    ? labels.filter((label): label is string => typeof label === "string" && label.trim() !== "")
    : [];
  return names.length > 0 ? names.join(", ") : t("hud.meeting.micInUse");
}

function pillIsBlank(state: string | undefined) {
  return state === undefined || state === "idle" || state === "exiting";
}

// Every state runs without native frost or native shadow. The HUD paints the
// same CSS surface and drop shadow as the agent HUD into a transparent gutter.
// Keeping idle/exiting frostless prevents any chrome swap during hide.
function usesFrostlessChrome(state: string | undefined) {
  void state;
  return true;
}

function canShowMeetingPrompt(state: string | undefined) {
  return state === undefined || state === "idle" || state === "meeting" || state === "exiting";
}

function parseEvent(payload: unknown): DictationHudEvent | undefined {
  try {
    if (typeof payload === "string") {
      return JSON.parse(payload) as DictationHudEvent;
    }
    if (payload && typeof payload === "object") {
      return payload as DictationHudEvent;
    }
  } catch {
    return undefined;
  }
  return undefined;
}

function cssPixelToken(token: string) {
  if (!hud) return 0;
  const value = Number.parseFloat(window.getComputedStyle(hud).getPropertyValue(token));
  return Number.isFinite(value) ? value : 0;
}

// The whole pill is a drag surface — press anywhere that isn't a control and
// move it. The explicit pointer path is more reliable than
// data-tauri-drag-region on the non-activating macOS panel.
hud?.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  if (event.target instanceof Element && event.target.closest("button")) return;
  event.preventDefault();
  void invoke("dictation_hud_start_drag").catch(() => {});
});

// While the collapse tween plays, window sizing is locked (syncWindowToPill
// awaits this) so nothing snaps the frame down mid-motion.
let collapseSettled: Promise<void> | undefined;
let cancelCollapse: (() => void) | undefined;
let collapseFallbackTimer: number | undefined;

function interruptCollapse() {
  cancelCollapse?.();
  cancelCollapse = undefined;
  collapseSettled = undefined;
  window.clearTimeout(collapseFallbackTimer);
  collapseFallbackTimer = undefined;
  hud?.classList.remove("hud-collapse-active");
  hud?.classList.remove("hud-collapse");
  if (hud) hud.style.width = "";
}

lifecycle.addCleanup(interruptCollapse);

// The stop moment: the shell width tweens down around the incoming spinner
// (CSS, see .hud.hud-collapse) while the waveform ghost fades in place. The
// native frame holds the old, larger size — it is transparent, so only the
// pill's motion is visible — and fits both slots once the tween settles.
async function collapseToSpinner(status: StatusText) {
  if (!hud || lifecycle.signal.aborted) return;
  const fromWidth = hud.getBoundingClientRect().width;
  const previousState = hud.dataset.state;
  hud.classList.add("hud-snap");
  hud.dataset.state = "transcribing";
  const collapsedPillSize = hud.getBoundingClientRect().width;
  hud.dataset.state = previousState;
  hud.classList.remove("hud-snap");
  if (prefersReducedMotion() || collapsedPillSize <= 0 || fromWidth <= collapsedPillSize) {
    // Reduced motion, or nothing measurable to tween (jsdom): atomic swap.
    const transition = setHud("transcribing", status);
    if (!transition.changed) return;
    await showHud(showOptionsForTransition(transition));
    return;
  }

  // Prime both sides of the morph while the listening waveform still owns the
  // visible state. The previous sequence changed data-state first, so the
  // waveform was already display:none and the spinner was already renderable
  // by the time this class arrived. WebKit therefore had no painted "from"
  // frame to animate on the first collapse.
  hud.style.width = `${fromWidth}px`;
  hud.classList.add("hud-collapse");
  const transition = setHud("transcribing", status);
  if (!transition.changed) {
    hud.classList.remove("hud-collapse");
    hud.style.width = "";
    return;
  }
  hud.offsetWidth;
  hud.classList.add("hud-collapse-active");
  hud.style.width = `${collapsedPillSize}px`;
  // Settle on the width transition actually finishing, not a timer: WebKit
  // throttles JS timers on this non-key panel, and a late timer left the
  // collapse class (and the window-sync lock) stuck for seconds. The timer
  // stays only as a generous fallback for environments with no transitions.
  const settled = new Promise<void>((resolve) => {
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      window.clearTimeout(collapseFallbackTimer);
      collapseFallbackTimer = undefined;
      hud?.removeEventListener("transitionend", onTransitionEnd);
      if (cancelCollapse === finish) cancelCollapse = undefined;
      resolve();
    };
    const onTransitionEnd = (event: TransitionEvent) => {
      if (event.target === hud && event.propertyName === "width") finish();
    };
    hud?.addEventListener("transitionend", onTransitionEnd);
    collapseFallbackTimer = window.setTimeout(finish, COLLAPSE_SETTLE_FALLBACK_MS);
    cancelCollapse = finish;
  });
  collapseSettled = settled;
  await settled;
  if (lifecycle.signal.aborted) return;
  if (collapseSettled === settled) collapseSettled = undefined;
  hud.classList.remove("hud-collapse-active");
  hud.classList.remove("hud-collapse");
  hud.style.width = "";
  const state = hud.dataset.state;
  if (state === "transcribing" || state === "pasting") {
    await syncWindowToPill({ animate: false });
  }
}

stopButton?.addEventListener("click", async (event) => {
  event.preventDefault();
  setStopHover(false);
  if (hud?.dataset.state === "listening") {
    void collapseToSpinner(transcribingStatus);
  }
  try {
    await invoke("dictation_helper_command", {
      command: correlatedHelperCommand("stop_and_paste"),
    });
  } catch {
    if (lifecycle.signal.aborted) return;
    if (hud?.dataset.state === "transcribing") {
      interruptCollapse();
      const transition = setHud("listening", listeningStatus);
      await showHud(showOptionsForTransition(transition));
      triggerShake();
    }
  }
});

// Cancel makes the current take terminal before asking the helper to discard
// it. Work that has not started is suppressed; a provider request already
// in flight may still complete. The helper answers with recording_discarded,
// which is the
// authoritative confirmation that capture actually stopped. Keep the
// listening HUD visible if the command write fails so an active recording is
// never concealed behind a dismissed panel.
async function cancelDictation() {
  if (hud?.dataset.state !== "listening") return;
  setStopHover(false);
  setCancelHover(false);
  setCancelTooltipHover(false);
  try {
    await invoke("dictation_helper_command", {
      command: correlatedHelperCommand("discard_recording"),
    });
  } catch {
    if (!lifecycle.signal.aborted) triggerShake();
  }
}

cancelButton?.addEventListener("click", async (event) => {
  event.preventDefault();
  event.stopPropagation();
  await cancelDictation();
});

// In-app the Escape press is captured globally by the dictation helper (the
// HUD panel never becomes key, so this webview sees no keystrokes); this
// listener covers the standalone browser page, where the demo driver runs
// without that native path.
window.addEventListener(
  "keydown",
  (event) => {
    if (event.key !== "Escape" || hud?.dataset.state !== "listening") return;
    event.preventDefault();
    event.stopPropagation();
    void cancelDictation();
  },
  { capture: true, signal: lifecycle.signal },
);

meetingStartButton?.addEventListener("click", async (event) => {
  event.preventDefault();
  event.stopPropagation();
  if (hud?.dataset.state !== "meeting") return;

  meetingStartButton.disabled = true;
  try {
    if (meetingVideoSelected && meetingVideoWindow) {
      // Native capture validates the opted-in id and persists a video failure
      // if it disappeared, while preserving the audio recording.
      const windowId = meetingVideoWindow.windowId;
      // Queue immediately: the helper revalidates after audio starts.
      await invoke("queue_meeting_start_request", { video: { windowId } });
    } else {
      await invoke("queue_meeting_start_request");
    }
  } catch {
    if (lifecycle.signal.aborted) return;
    // The request was not durably queued, so leave the prompt available for
    // another click instead of hiding a failed action.
    meetingStartButton.disabled = false;
    return;
  }
  if (lifecycle.signal.aborted) return;
  meetingPromptSuppressed = true;
  clearMeetingPromptTimer();
  void hideHud().finally(() => {
    meetingStartButton.disabled = false;
  });
});

meetingDismissButton?.addEventListener("click", (event) => {
  event.preventDefault();
  event.stopPropagation();
  if (hud?.dataset.state !== "meeting") return;

  // Same semantics as letting the prompt time out: stay quiet for the rest
  // of this meeting (detection heartbeats keep arriving while the call is
  // live) and prompt again once it clears.
  meetingPromptSuppressed = true;
  clearMeetingPromptTimer();
  void hideHud();
});

lifecycle.trackUnlisten(
  listen("dictation-event", async (event) => {
    const previousTransition = hudTransitionId;
    const handling = handleDictationEventPayload(event.payload);
    // Only an applied lifecycle transition supersedes the startup snapshot.
    // Diagnostics, configuration and rejected stale events must not discard it.
    if (hudTransitionId !== previousTransition) receivedLiveDictationEvent = true;
    await handling;
  }),
);

lifecycle.trackUnlisten(
  listen("meeting-detection-event", async (event) => {
    await handleMeetingDetectionEventPayload(event.payload);
  }),
);

lifecycle.trackUnlisten(
  listen<boolean>("hud-stop-hover", (event) => {
    setStopHover(Boolean(event.payload));
  }),
);

lifecycle.trackUnlisten(
  listen<boolean>("hud-cancel-hover", (event) => {
    setCancelHover(Boolean(event.payload));
  }),
);

lifecycle.trackUnlisten(
  listen<boolean>("hud-cancel-tooltip-hover", (event) => {
    setCancelTooltipHover(Boolean(event.payload));
  }),
);

lifecycle.trackUnlisten(
  listen<boolean>("hud-dismiss-hover", (event) => {
    setDismissHover(Boolean(event.payload));
  }),
);

lifecycle.trackUnlisten(
  listen<boolean>("hud-record-hover", (event) => {
    setRecordHover(Boolean(event.payload));
  }),
);

// Cold-start companion to the await in syncWindowToPill: the Diatype load
// may only BEGIN once the prompt first paints text, after the measurement.
// When the faces land, re-fit the window to whatever is showing.
if (typeof document.fonts?.ready?.then === "function") {
  void document.fonts.ready.then(() => {
    if (lifecycle.signal.aborted) return;
    const state = hud?.dataset.state;
    if (state && state !== "idle" && state !== "exiting") {
      void syncWindowToPill();
    }
  });
}

// Local mirrors of the Tauri listeners, same as the agent HUD page:
// only the dev-only demo drivers dispatch these window events (standalone
// page, no bridge), so production builds skip the dead listeners.
if (import.meta.env.DEV) {
  window.addEventListener(
    "dictation-event",
    (event) => {
      void handleDictationEventPayload((event as CustomEvent).detail);
    },
    { signal: lifecycle.signal },
  );

  window.addEventListener(
    "meeting-detection-event",
    (event) => {
      void handleMeetingDetectionEventPayload((event as CustomEvent).detail);
    },
    { signal: lifecycle.signal },
  );
}

lifecycle.addCleanup(subscribeToOnboardingComplete(showPendingMeetingPromptAfterOnboarding));

// Language change from the main window: re-render the visible state in place
// and re-fit the native frame to the new label widths.
function rerenderForLocale() {
  applyStaticLabels();
  const state = hud?.dataset.state;
  const status = resolveStatus(currentStatus);
  if (statusText) statusText.textContent = status;
  if (errorText && state === "error") errorText.textContent = status;
  if (meetingAppLabel && state === "meeting" && !meetingVideoWindow) {
    meetingAppLabel.textContent = meetingAppLine(meetingAppLabels);
  }
  if (state && state !== "idle" && state !== "exiting") {
    void syncWindowToPill({ animate: false });
  }
}

lifecycle.addCleanup(subscribeInterfaceLocale(rerenderForLocale));

// Console drivers for this page when served standalone in a browser:
// __dictationHud("listening") drives the dictation pill, __meetingHud(
// "detected") drives the meeting-detection prompt. See lib/dictation-hud-demo.ts
// and lib/meeting-hud-demo.ts.
if (import.meta.env.DEV) {
  void import("./lib/dictation-hud-demo").then(({ registerDictationHudDemo }) => {
    if (!lifecycle.signal.aborted) registerDictationHudDemo({ local: true });
  });
  void import("./lib/meeting-hud-demo").then(({ registerMeetingHudDemo }) => {
    if (!lifecycle.signal.aborted) registerMeetingHudDemo({ local: true });
  });
}

void invoke<string | undefined>("latest_dictation_event")
  .then((payload) => {
    if (lifecycle.signal.aborted || receivedLiveDictationEvent) return;
    if (payload) {
      const snapshot = parseEvent(payload);
      if (snapshot) activeDictationTakeId = dictationTakeId(snapshot);
      return handleDictationEventPayload(payload);
    }
  })
  .catch(() => {});
