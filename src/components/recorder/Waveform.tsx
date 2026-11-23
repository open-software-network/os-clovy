import { useEffect, useRef } from "react";
import { useT } from "../../i18n";
import type { AudioLevelDto } from "../../lib/tauri";
import { useRecordingTelemetryLevel } from "../../lib/recording-telemetry-store";
import {
  createBarMeter,
  IDLE_LEVEL,
  LIVE_WAVE_OPTIONS,
  RECORDER_BAR_COUNT,
  RECORDER_BAR_HISTORY_OFFSETS,
  RECORDER_BAR_WEIGHTS,
  withWaveLayers,
} from "../../lib/audio-meter";
import { visualPeakScale } from "../../lib/recorder-levels";

// Shaping + source-mixing helpers moved to ../../lib/recorder-levels so the
// vanilla-TS meeting HUD can share them; re-exported here for existing
// importers (RecorderBar, tests).
export {
  combineAudioLevels,
  combineSourceAudioLevels,
  meterLevelForSources,
  scaleAudioLevel,
  SOURCE_VISUAL_GAIN,
  visualPeakScale,
} from "../../lib/recorder-levels";

type WaveformProps = {
  level: AudioLevelDto;
  sessionId?: string;
  // Whether recording is live. The idle carrier shimmer only travels while
  // active; when paused the bars settle and hold (CSS also dims them).
  active?: boolean;
};

// How many of the freshest `recentPeaks` to coalesce per poll — sized to ~the
// 50ms poll window at the default audio buffer (~11ms/callback ≈ 4–5 peaks, +1
// headroom for smaller buffers). Deliberately a short window, not the full
// deque, so the bars die down immediately. See the push effect below.
const POLL_WINDOW_PEAKS = 6;

export function Waveform({ level, sessionId, active = true }: WaveformProps) {
  const t = useT();
  const telemetryLevel = useRecordingTelemetryLevel(sessionId, level);
  const refs = useRef<Array<HTMLSpanElement | null>>([]);
  // Shares the dictation HUD's synthesis + ballistics AND its travelling-wave
  // motion, with the recorder's own taller 7-bar layout and peak-based shaping.
  const meterRef = useRef(
    createBarMeter(
      RECORDER_BAR_COUNT,
      RECORDER_BAR_WEIGHTS,
      RECORDER_BAR_HISTORY_OFFSETS,
      LIVE_WAVE_OPTIONS,
    ),
  );
  // Read the latest `active` from inside the rAF loop without re-subscribing it.
  const activeRef = useRef(active);
  activeRef.current = active;

  // Feed a sample into the meter on every poll (keyed on the level prop, not the
  // shaped value — silence collapses to a constant 0, and we still want the
  // history ring to advance each poll). The rAF loop animates the bars toward
  // it (fast attack, smooth release, snap-to-zero on silence).
  useEffect(() => {
    // Model the HUD's signal: coalesce the peak over roughly the poll window so
    // transients between polls aren't missed. `recentPeaks` is a fixed ~24-entry
    // deque (~260ms at the default audio buffer), NOT the poll window — maxing
    // the whole thing would reintroduce a long peak-hold and a mushy die-down,
    // so we max only the freshest few entries (~the 50ms poll at typical buffer
    // sizes). The cumulative `peak` is a since-start max (frozen), so it's only
    // the empty-history fallback.
    const recent = telemetryLevel.recentPeaks;
    const raw =
      recent.length > 0 ? Math.max(...recent.slice(-POLL_WINDOW_PEAKS)) : telemetryLevel.peak;
    meterRef.current.pushLevel(visualPeakScale(raw));
  }, [telemetryLevel]);

  useEffect(() => {
    const meter = meterRef.current;
    let raf = 0;
    const tick = (now: number) => {
      meter.step();
      let speech = 0;
      for (let i = 0; i < RECORDER_BAR_COUNT; i++) {
        speech = Math.max(speech, meter.displayed[i]);
      }
      for (let i = 0; i < RECORDER_BAR_COUNT; i++) {
        const el = refs.current[i];
        if (!el) continue;
        const value = activeRef.current
          ? withWaveLayers(meter.displayed[i], i, now, speech, RECORDER_BAR_COUNT)
          : meter.displayed[i];
        el.style.setProperty("--level", value.toFixed(3));
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  return (
    <div className="waveform" aria-label={t("recorder.audioActivity")}>
      {Array.from({ length: RECORDER_BAR_COUNT }, (_, index) => (
        <span
          key={index}
          style={{ ["--level" as string]: IDLE_LEVEL }}
          ref={(el) => {
            refs.current[index] = el;
          }}
        />
      ))}
    </div>
  );
}
