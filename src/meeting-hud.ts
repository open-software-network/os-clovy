import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  createBarMeter,
  IDLE_LEVEL,
  LIVE_WAVE_OPTIONS,
  RECORDER_BAR_COUNT,
  RECORDER_BAR_HISTORY_OFFSETS,
  RECORDER_BAR_WEIGHTS,
  withWaveLayers,
} from "./lib/audio-meter";
import { meterLevelForSources, visualPeakScale } from "./lib/recorder-levels";
import {
  RECORDING_TELEMETRY_EVENT,
  type MeetingEndStatus,
  type RecordingStatusDto,
  type RecordingTelemetryDto,
} from "./lib/tauri";
import { MEETING_END_STATE_EVENT } from "./lib/events";
import { installNativeContextMenuGuard } from "./lib/native-context-menu";
import { subscribeBrand } from "./lib/brand";
import { createHudLifecycle } from "./lib/hud-lifecycle";
import { subscribeInterfaceLocale, subscribeInterfaceLocaleAcrossWindows, t } from "./i18n";
import "./styles/meeting-hud.css";

const lifecycle = createHudLifecycle();

// Recolor this HUD window to the selected accent and keep it live-synced.
lifecycle.trackUnlisten(subscribeBrand());
// Follow the interface language chosen in the main window (sets <html lang>).
lifecycle.addCleanup(subscribeInterfaceLocaleAcrossWindows());

const HAS_TAURI_BRIDGE = "__TAURI_INTERNALS__" in window;

// Absent on the standalone browser page (no Tauri bridge), where the demo
// driver exercises the pill — getCurrentWindow() throws there. Same guard as
// hud.ts.
const appWindow = (() => {
  try {
    return getCurrentWindow();
  } catch {
    return undefined;
  }
})();
const pill = document.querySelector<HTMLDivElement>("#mhud");
const bars = Array.from(document.querySelectorAll<HTMLElement>(".mhud-bar"));
const meetingEndKeep = document.querySelector<HTMLButtonElement>("#mhud-end-keep");
const meetingEndStop = document.querySelector<HTMLButtonElement>("#mhud-end-stop");
const meetingEndSeconds = document.querySelector<HTMLElement>("#mhud-end-seconds");
const meetingEndTitle = document.querySelector<HTMLElement>(".mhud-end-title");

// Replace a control's own text while keeping its child elements (the Stop
// button carries the countdown seconds span next to its label).
function setOwnText(element: HTMLElement | null, text: string) {
  if (!element) return;
  const node = Array.from(element.childNodes).find(
    (child) => child.nodeType === Node.TEXT_NODE && child.textContent?.trim(),
  );
  if (node) node.textContent = text;
  else element.prepend(document.createTextNode(text));
}

function pillAriaLabel() {
  if (pill?.dataset.mode === "meeting-end") return t("hud.meetingEnd.title");
  return pill?.dataset.state === "paused"
    ? t("hud.recording.paused")
    : t("hud.recording.recording");
}

// The static page chrome from meeting-hud.html, in the current language.
function applyStaticLabels() {
  document.title = t("hud.recording.windowTitle");
  pill?.setAttribute("aria-label", pillAriaLabel());
  if (meetingEndTitle) meetingEndTitle.textContent = t("hud.meetingEnd.title");
  setOwnText(meetingEndStop, t("hud.meetingEnd.stop"));
  meetingEndStop?.setAttribute("aria-label", t("hud.meetingEnd.stopLabel"));
  setOwnText(meetingEndKeep, t("hud.meetingEnd.keep"));
}

/** Must track MEETING_END_COUNTDOWN_MS in meeting_detection.rs — the status
 * event only carries the deadline, not the countdown's full length. */
const MEETING_END_COUNTDOWN_MS = 15_000;

lifecycle.addCleanup(installNativeContextMenuGuard());

// Shares the dictation HUD's + recorder bar's synthesis, ballistics, and
// travelling-wave motion so all waveforms move identically.
const meter = createBarMeter(
  RECORDER_BAR_COUNT,
  RECORDER_BAR_WEIGHTS,
  RECORDER_BAR_HISTORY_OFFSETS,
  LIVE_WAVE_OPTIONS,
);

// Coalesce the freshest peaks per telemetry sample, matching the in-app
// recorder (Waveform.tsx) so transients between native pushes aren't missed.
const TELEMETRY_WINDOW_PEAKS = 6;

let recording = false;
let meetingEndStatus: MeetingEndStatus | null = null;
let meetingEndTick: number | undefined;
let meetingEndEventRevision = 0;

lifecycle.addCleanup(() => {
  if (meetingEndTick !== undefined) window.clearInterval(meetingEndTick);
});

function applyStatus(status: RecordingStatusDto | RecordingTelemetryDto) {
  const paused = status.state === "paused";
  recording = status.state === "recording";

  if (pill) {
    pill.dataset.state = paused ? "paused" : "recording";
    pill.setAttribute(
      "aria-label",
      paused ? t("hud.recording.paused") : t("hud.recording.recording"),
    );
  }

  // status.level is mic-only; status.sources carries mic+system when present.
  const level = meterLevelForSources(status.level, status.sources);
  const recent = level.recentPeaks;
  const raw = recent.length > 0 ? Math.max(...recent.slice(-TELEMETRY_WINDOW_PEAKS)) : level.peak;
  meter.pushLevel(visualPeakScale(raw));
}

function updateMeetingEndDrain() {
  if (!meetingEndStop || meetingEndStatus?.phase !== "countdown") return;
  const expiresAt = meetingEndStatus.expiresAtMs ?? Date.now();
  const remainingMs = Math.max(0, expiresAt - Date.now());
  meetingEndStop.style.setProperty(
    "--meeting-end-remaining",
    String(Math.min(1, remainingMs / MEETING_END_COUNTDOWN_MS)),
  );
  if (meetingEndSeconds) {
    meetingEndSeconds.textContent = t("hud.meetingEnd.seconds", {
      seconds: Math.ceil(remainingMs / 1_000),
    });
  }
}

function applyMeetingEndStatus(status: MeetingEndStatus | null) {
  meetingEndStatus = status;
  const countdown = status?.phase === "countdown";
  if (meetingEndTick !== undefined) {
    window.clearInterval(meetingEndTick);
    meetingEndTick = undefined;
  }
  if (pill) {
    // Mode swaps snap: the native window resize is instant, so a width tween
    // in either direction would slide the tint out from under the frost.
    pill.classList.add("mhud-snap");
    if (countdown) {
      pill.dataset.mode = "meeting-end";
      pill.removeAttribute("role");
      pill.removeAttribute("tabindex");
      pill.setAttribute("aria-label", t("hud.meetingEnd.title"));
    } else {
      delete pill.dataset.mode;
      pill.setAttribute("role", "button");
      pill.setAttribute("tabindex", "0");
      pill.setAttribute("aria-label", t("hud.recording.recording"));
    }
    // Let the snapped state paint before the transition comes back (same
    // two-frame dance as applyZone).
    lifecycle.requestAnimationFrame(() => {
      lifecycle.requestAnimationFrame(() => pill.classList.remove("mhud-snap"));
    });
  }
  meetingEndKeep?.toggleAttribute("disabled", !countdown);
  meetingEndStop?.toggleAttribute("disabled", !countdown);
  if (countdown) {
    // Snap the drain to the current remaining fraction (no sweep from a
    // stale value when a countdown restarts), then step it once a second —
    // the 1s linear transition in CSS carries it smoothly between ticks.
    meetingEndStop?.classList.add("mhud-end-drain-snap");
    updateMeetingEndDrain();
    lifecycle.requestAnimationFrame(() => {
      lifecycle.requestAnimationFrame(() =>
        meetingEndStop?.classList.remove("mhud-end-drain-snap"),
      );
    });
    meetingEndTick = window.setInterval(updateMeetingEndDrain, 1_000);
  }
  void invoke("meeting_hud_set_end_prompt_expanded", { expanded: countdown }).catch(() => {});
}

// Orientation triad: parked in the left or right third of the screen the pill
// stands vertical (mark above the waveform); the middle third lies flat. Rust
// owns the turn itself — a Core Animation transform on the native contentView
// that carries frost, tint, and this whole DOM in one move. Our only job is
// the counter-turn: flip `data-orient` so CSS spins the bars back the other
// way (same duration/curve) and the waveform keeps reading left-to-right.
function applyZone(payload: { vertical: boolean; animate: boolean }) {
  if (!pill) return;
  if (!payload.animate) pill.classList.add("mhud-snap");
  pill.dataset.orient = payload.vertical ? "vertical" : "horizontal";
  if (!payload.animate) {
    // Let the snapped state paint before the transition comes back.
    lifecycle.requestAnimationFrame(() => {
      lifecycle.requestAnimationFrame(() => pill.classList.remove("mhud-snap"));
    });
  }
}

function startBarLoop() {
  const tick = (now: number) => {
    meter.step();
    // Overall loudness = the tallest bar right now; drives the speech wave.
    let speech = 0;
    for (let i = 0; i < bars.length; i++) {
      speech = Math.max(speech, meter.displayed[i]);
    }
    for (let i = 0; i < bars.length; i++) {
      const value = recording
        ? withWaveLayers(meter.displayed[i], i, now, speech, bars.length)
        : meter.displayed[i];
      bars[i].style.setProperty("--level", value.toFixed(3));
    }
    lifecycle.requestAnimationFrame(tick);
  };
  lifecycle.requestAnimationFrame(tick);
}

function resetBars() {
  for (const bar of bars) {
    bar.style.setProperty("--level", IDLE_LEVEL.toFixed(3));
  }
}

// Focus the main window from Rust (reliable app activation — clicking a
// non-activating panel won't bring a backgrounded app forward on its own); Rust
// then emits the action React uses to land back on the recording note.
function reopenClovy() {
  void invoke("meeting_hud_reopen").catch(() => {});
}

// One surface, two gestures: a press that moves past a small threshold drags
// the window; a press that stays put is a click → reopen Clovy. Handlers live
// on the document, not the pill: the native quarter-turn rotates pixels but
// not DOM hit-testing, so when the pill stands vertical a click on it lands in
// the DOM's gutter. The window is barely bigger than the pill, so document-
// wide is the honest target.
const DRAG_THRESHOLD_PX = 4;
let pressStart: { x: number; y: number } | undefined;
let dragging = false;

document.addEventListener(
  "pointerdown",
  (event) => {
    if (event.button !== 0) return;
    if (pill?.dataset.mode === "meeting-end") return;
    pressStart = { x: event.screenX, y: event.screenY };
    dragging = false;
  },
  { signal: lifecycle.signal },
);

document.addEventListener(
  "pointermove",
  (event) => {
    if (!pressStart || dragging) return;
    const dx = event.screenX - pressStart.x;
    const dy = event.screenY - pressStart.y;
    if (Math.hypot(dx, dy) > DRAG_THRESHOLD_PX) {
      dragging = true;
      // Native drag takes over the gesture; pointerup won't fire on the element,
      // so `dragging` stays true and suppresses the click below.
      void appWindow?.startDragging().catch(() => {});
    }
  },
  { signal: lifecycle.signal },
);

document.addEventListener(
  "pointerup",
  (event) => {
    if (event.button !== 0) return;
    const wasClick = pill?.dataset.mode !== "meeting-end" && !!pressStart && !dragging;
    pressStart = undefined;
    if (wasClick) reopenClovy();
  },
  { signal: lifecycle.signal },
);

document.addEventListener(
  "keydown",
  (event) => {
    if (pill?.dataset.mode === "meeting-end") return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      reopenClovy();
    }
  },
  { signal: lifecycle.signal },
);

meetingEndKeep?.addEventListener(
  "click",
  (event) => {
    event.preventDefault();
    event.stopPropagation();
    const sessionId = meetingEndStatus?.sessionId;
    if (!sessionId || meetingEndStatus?.phase !== "countdown") return;
    meetingEndKeep.disabled = true;
    meetingEndStop?.toggleAttribute("disabled", true);
    void invoke("keep_meeting_recording", { sessionId }).catch(() => {
      meetingEndKeep.disabled = false;
      meetingEndStop?.toggleAttribute("disabled", false);
    });
  },
  { signal: lifecycle.signal },
);

meetingEndStop?.addEventListener(
  "click",
  (event) => {
    event.preventDefault();
    event.stopPropagation();
    const sessionId = meetingEndStatus?.sessionId;
    if (!sessionId || meetingEndStatus?.phase !== "countdown") return;
    meetingEndStop.disabled = true;
    meetingEndKeep?.toggleAttribute("disabled", true);
    void invoke("queue_meeting_end_finish_request", { sessionId }).catch(() => {
      meetingEndStop.disabled = false;
      meetingEndKeep?.toggleAttribute("disabled", false);
    });
  },
  { signal: lifecycle.signal },
);

lifecycle.trackUnlisten(
  listen<RecordingTelemetryDto>(RECORDING_TELEMETRY_EVENT, (event) => {
    if (event.payload) applyStatus(event.payload);
  }),
);

lifecycle.trackUnlisten(
  listen<{ vertical: boolean; animate: boolean }>("meeting-hud-zone", (event) => {
    if (event.payload) applyZone(event.payload);
  }),
);

lifecycle.trackUnlisten(
  listen<MeetingEndStatus | null>(MEETING_END_STATE_EVENT, (event) => {
    meetingEndEventRevision += 1;
    applyMeetingEndStatus(event.payload ?? null);
  }),
);

// Local mirrors of the Tauri listeners, same as the dictation HUD page:
// only the dev-only demo driver dispatches these window events (standalone
// page, no bridge), so production builds skip the dead listeners.
if (import.meta.env.DEV) {
  window.addEventListener(
    "meeting-hud-status",
    (event) => {
      const status = (event as CustomEvent<RecordingStatusDto>).detail;
      if (status) applyStatus(status);
    },
    { signal: lifecycle.signal },
  );

  window.addEventListener(
    "meeting-hud-zone",
    (event) => {
      const payload = (event as CustomEvent<{ vertical: boolean; animate: boolean }>).detail;
      if (payload) applyZone(payload);
    },
    { signal: lifecycle.signal },
  );

  window.addEventListener(
    MEETING_END_STATE_EVENT,
    (event) => {
      meetingEndEventRevision += 1;
      applyMeetingEndStatus((event as CustomEvent<MeetingEndStatus | null>).detail ?? null);
    },
    { signal: lifecycle.signal },
  );
}

applyStaticLabels();
// Re-label the visible state when the language changes.
lifecycle.addCleanup(
  subscribeInterfaceLocale(() => {
    applyStaticLabels();
    updateMeetingEndDrain();
  }),
);

resetBars();
startBarLoop();

// Console driver for this page when served standalone in a browser:
// __recordingHud("recording") etc. See lib/recording-hud-demo.ts.
if (import.meta.env.DEV) {
  void import("./lib/recording-hud-demo").then(({ registerRecordingHudDemo }) => {
    const demo = registerRecordingHudDemo({ local: !HAS_TAURI_BRIDGE });
    lifecycle.addCleanup(demo.dispose);
  });
}

// Paint immediately if a recording is already live when this view appears.
void invoke<RecordingStatusDto | null>("meeting_hud_latest_status")
  .then((status) => {
    if (status) applyStatus(status);
  })
  .catch(() => {});

const revisionBeforeInitialMeetingEndRead = meetingEndEventRevision;
void invoke<MeetingEndStatus | null>("pending_meeting_end_status")
  .then((status) => {
    if (meetingEndEventRevision === revisionBeforeInitialMeetingEndRead) {
      applyMeetingEndStatus(status);
    }
  })
  .catch(() => {});
