// Dev-only console driver for the recorder-area notices that appear inside a
// meeting note while it is recording:
//
//   window.__recordNoticesDemo("consent")          park the consent reminder
//   window.__recordNoticesDemo("warning")          park a recording source warning
//   window.__recordNoticesDemo("warning", "…")     ...with a custom warning message
//   window.__recordNoticesDemo("mic")              park the mic-blocked notice (no recording)
//   window.__recordNoticesDemo("clear")            tear the demo down, back to real state
//
// The consent reminder and the recording source warning normally sit behind the
// recorder bar's reveal/auto-hide timers, so there is no way to hold them on
// screen for styling review without a real recording. This driver pushes a
// synthetic recording status straight into the reducer (under a sentinel
// session id the status poll and the pause/resume/finish handlers skip, so no
// backend call fires) and pins the consent reminder past its timers, so the
// notices can be inspected in the browser sandbox or `pnpm tauri:dev`. It parks on the
// selected note, seeding a minimal in-memory note when none is selected.
//
// Because it drives the same reducer recording status, every state-mutating
// command refuses while a real recording is live, rather than stomping it and
// stranding the backend recording with no pause/resume/finish controls.
//
// Mirrors the sibling dev drivers in
// lib/processing-progress-demo.ts and lib/global-recorder-demo.ts.
//
// Never bundled in production: App gates the dynamic import on
// import.meta.env.DEV.

import { RECORD_NOTICES_DEMO_SESSION_ID } from "../app/processing-demo-ids";
import type { NoteDto, RecordingStatusDto } from "./tauri";

const DEMO_NOTE_ID = "dev-record-notices-demo-note";

// Mirrors the real microphone stall warning (src-tauri audio/capture.rs) so the
// parked notice reads exactly like production.
const DEFAULT_WARNING_MESSAGE =
  "Microphone input stopped unexpectedly. Audio after this point may be missing.";

// Match the real active telemetry cadence so the parked in-note recorder has
// production waveform timing during visual review.
const TICK_MS = 50;

type Variant = "consent" | "warning";

export type RecordNoticesDemoApi = {
  /** Tear down timers and remove the window hook. */
  dispose: () => void;
  /** Tear the demo down and restore real recorder state (also used when the
   * recorder bar's finish button is pressed on the demo session). */
  clear: () => void;
};

const HELP = [
  "Recorder notices demo (meeting note recorder bar):",
  '  __recordNoticesDemo("consent")        park the consent reminder',
  '  __recordNoticesDemo("warning")        park a recording source warning',
  '  __recordNoticesDemo("warning", "…")   ...with a custom warning message',
  '  __recordNoticesDemo("mic")            park the mic-blocked notice (no recording)',
  '  __recordNoticesDemo("clear")          tear the demo down, back to real state',
  "",
  "Parks the recorder notices without a real recording, on the selected note",
  "(seeds one if none is selected). Dev only.",
].join("\n");

// Shown when a command that would mutate recorder state runs while a real
// recording is live: the driver refuses rather than stomping the reducer's
// recording status (which would strand the backend recording with no controls).
const REAL_RECORDING_REFUSAL =
  "A real recording is in progress. Finish it before running the demo.";

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

// A minimal in-memory note to park on when nothing is selected. "recording"
// keeps the synthetic note out of native processing and
// reads truthfully for a note being recorded.
function buildDemoNote(): NoteDto {
  const timestamp = "2026-06-30T15:04:00.000Z";
  return {
    id: DEMO_NOTE_ID,
    title: "Weekly product sync",
    preview: "Recording in progress",
    processingStatus: "recording",
    folderIds: [],
    createdAt: timestamp,
    updatedAt: timestamp,
    activeTab: "notes",
  };
}

export function registerRecordNoticesDemo({
  seedNote,
  setStatus,
  setConsentPinned,
  setMicOverride,
  getSelectedNoteId,
  hasRealRecording,
}: {
  /** Add the minimal demo note and select it on the meeting-notes view. */
  seedNote: (note: NoteDto) => void;
  /** Push (or clear) the synthetic recording status the recorder bar reads. */
  setStatus: (status: RecordingStatusDto | null) => void;
  /** Pin the consent reminder past its reveal/auto-hide timers. */
  setConsentPinned: (pinned: boolean) => void;
  /** Force (or release) the mic-blocked notice, independent of real TCC state. */
  setMicOverride: (blocked: boolean | null) => void;
  /** The id of the currently selected note, or undefined if none. */
  getSelectedNoteId: () => string | undefined;
  /** True when a real (non-sentinel) recording is live; guards the driver from
   * stomping the reducer's recording status out from under the backend. */
  hasRealRecording: () => boolean;
}): RecordNoticesDemoApi {
  let timer: number | undefined;
  let phase = 0;
  let elapsedMs = 0;
  let noteId = DEMO_NOTE_ID;
  let variant: Variant = "consent";
  let warningMessage = DEFAULT_WARNING_MESSAGE;

  function stopTicker() {
    if (timer !== undefined) {
      window.clearInterval(timer);
      timer = undefined;
    }
  }

  // Resolve the note to park on: the selected note, or a seeded in-memory one.
  function resolveTargetNote(): string {
    const selected = getSelectedNoteId();
    if (selected) return selected;
    seedNote(buildDemoNote());
    return DEMO_NOTE_ID;
  }

  function buildStatus(level: number): RecordingStatusDto {
    // recentPeaks feeds the waveform's coalescing tail; a slight per-entry
    // falloff reads as a freshest-first window like the real poll.
    const recentPeaks = Array.from({ length: 6 }, (_, i) =>
      clamp01(level + (Math.random() - 0.5) * 0.2 - i * 0.02),
    );
    return {
      sessionId: RECORD_NOTICES_DEMO_SESSION_ID,
      noteId,
      sourceMode: "microphonePlusSystem",
      state: "recording",
      elapsedMs,
      level: { peak: level, rms: level * 0.7, recentPeaks },
      silenceWarning: false,
      bytesWritten: 0,
      warnings:
        variant === "warning"
          ? [
              {
                source: "microphone",
                code: "microphone_stream_stalled",
                message: warningMessage,
              },
            ]
          : undefined,
    };
  }

  function pushStatus() {
    const carrier = 0.42 + 0.3 * Math.sin(phase * 0.18);
    const jitter = (Math.random() - 0.5) * 0.22;
    setStatus(buildStatus(clamp01(carrier + jitter)));
  }

  // Keep the recorder bar alive: advance the elapsed clock and repaint the
  // waveform on a steady tick, exactly like a real recording poll.
  function startTicker() {
    stopTicker();
    pushStatus();
    timer = window.setInterval(() => {
      phase += 1;
      elapsedMs += TICK_MS;
      pushStatus();
    }, TICK_MS);
  }

  function parkRecording(next: Variant, message?: string) {
    phase = 0;
    elapsedMs = 0;
    variant = next;
    warningMessage = message ?? DEFAULT_WARNING_MESSAGE;
    setMicOverride(null);
    // The consent reminder only renders when there are no warnings, so pinning
    // it for the warning variant is harmless — but keep it off for clarity.
    setConsentPinned(next === "consent");
    noteId = resolveTargetNote();
    startTicker();
  }

  function parkMic() {
    stopTicker();
    phase = 0;
    elapsedMs = 0;
    // The mic-blocked notice and the recorder bar are mutually exclusive in the
    // UI (micDenied && !recordingForNote), so clear any demo recording first.
    setStatus(null);
    setConsentPinned(false);
    // Ensure a note is on screen to host the editor footer.
    if (!getSelectedNoteId()) seedNote(buildDemoNote());
    setMicOverride(true);
  }

  function clear() {
    stopTicker();
    phase = 0;
    elapsedMs = 0;
    setStatus(null);
    setConsentPinned(false);
    setMicOverride(null);
  }

  const hook = (command?: string, arg?: string) => {
    // Every state-mutating command refuses while a real recording is live: the
    // demo drives the same reducer status, so parking or clearing here would
    // strand the backend recording with no pause/resume/finish controls.
    if (
      (command === "consent" ||
        command === "warning" ||
        command === "mic" ||
        command === "clear") &&
      hasRealRecording()
    ) {
      return REAL_RECORDING_REFUSAL;
    }
    switch (command) {
      case "consent":
        parkRecording("consent");
        return 'Parked the consent reminder. __recordNoticesDemo("clear") to finish.';
      case "warning":
        parkRecording("warning", typeof arg === "string" ? arg : undefined);
        return 'Parked a recording source warning. __recordNoticesDemo("clear") to finish.';
      case "mic":
        parkMic();
        return 'Parked the mic-blocked notice. __recordNoticesDemo("clear") to finish.';
      case "clear":
        clear();
        return "Recorder notices demo cleared; back to real state.";
      default:
        return HELP;
    }
  };

  (window as unknown as Record<string, unknown>).__recordNoticesDemo = hook;

  function dispose() {
    stopTicker();
    delete (window as unknown as Record<string, unknown>).__recordNoticesDemo;
  }

  return { dispose, clear };
}
