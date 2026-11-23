import { IconPause } from "central-icons-filled/IconPause";
import { IconPlay } from "central-icons-filled/IconPlay";
import { IconStop } from "central-icons-filled/IconStop";
import { useT } from "../../i18n";
import type { RecordingStatusDto } from "../../lib/tauri";
import { useRecordingElapsedMs } from "../../lib/recording-telemetry-store";
import { combineSourceAudioLevels, Waveform } from "./Waveform";

type RecorderBarProps = {
  status: RecordingStatusDto;
  onPause: (sessionId: string) => void;
  onResume: (sessionId: string) => void;
  onDone: (sessionId: string) => void;
};

export function RecorderBar({ status, onPause, onResume, onDone }: RecorderBarProps) {
  const t = useT();
  const paused = status.state === "paused";
  const controlsEnabled = status.state === "recording" || status.state === "paused";
  const elapsedMs = useRecordingElapsedMs(status.sessionId, status.elapsedMs);
  // status.level is mic-only; status.sources carries mic+system when available.
  const meterLevel =
    status.sources && status.sources.length > 0
      ? combineSourceAudioLevels(status.sources)
      : status.level;
  const pauseLabel = paused
    ? t("recorder.resume")
    : status.state === "recording"
      ? t("recorder.pause")
      : status.state === "starting"
        ? t("recorder.starting")
        : t("recorder.finalizing");
  const stopLabel = controlsEnabled ? t("common.done") : t("recorder.working");

  return (
    <div className="recorder-bar" data-state={status.state}>
      <button
        type="button"
        className="recorder-icon-button"
        disabled={!controlsEnabled}
        onClick={() => (paused ? onResume(status.sessionId) : onPause(status.sessionId))}
        aria-label={pauseLabel}
        title={pauseLabel}
      >
        {paused ? <IconPlay size={14} /> : <IconPause size={14} />}
      </button>
      <div className="recorder-meter">
        <span className="elapsed">{formatElapsed(elapsedMs)}</span>
        <Waveform
          level={meterLevel}
          sessionId={status.sessionId}
          active={status.state === "recording"}
        />
      </div>
      <button
        type="button"
        className="recorder-stop"
        disabled={!controlsEnabled}
        onClick={() => onDone(status.sessionId)}
        aria-label={stopLabel}
        title={stopLabel}
      >
        <IconStop size={14} />
      </button>
    </div>
  );
}

export function formatElapsed(ms: number) {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`;
}
