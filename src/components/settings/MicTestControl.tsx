import { useEffect, useRef, useState } from "react";
import { type TFunction, useT } from "../../i18n";

export type MicTestState = "idle" | "recording" | "ready" | "error";

type MicTestControlProps = {
  state: MicTestState;
  level: number;
  /** When the recording started; the elapsed clock ticks here, not in its parent. */
  startedAt?: number;
  sampleSrc?: string;
  error?: string;
  playing: boolean;
  durationSeconds: number;
  onStart: () => void;
  onStartOver: () => void;
  onPlaybackError: () => void;
  onPlayingChange: (playing: boolean) => void;
};

/** The recording clock, re-rendered on its own 10 times a second. */
function MicTestTime({
  startedAt,
  durationSeconds,
}: {
  startedAt?: number;
  durationSeconds: number;
}) {
  const [elapsedMs, setElapsedMs] = useState(0);
  useEffect(() => {
    setElapsedMs(0);
    if (!startedAt) return;
    const interval = window.setInterval(() => {
      setElapsedMs(Date.now() - startedAt);
    }, 100);
    return () => window.clearInterval(interval);
  }, [startedAt]);
  return (
    <span className="settings-mic-test-time">
      {formatMicTestTime(elapsedMs, durationSeconds)} / {formatMicTestDuration(durationSeconds)}
    </span>
  );
}

export function MicTestControl({
  state,
  level,
  startedAt,
  sampleSrc,
  error,
  playing,
  durationSeconds,
  onStart,
  onStartOver,
  onPlaybackError,
  onPlayingChange,
}: MicTestControlProps) {
  const t = useT();
  return (
    <div className="settings-row settings-row-mic-test">
      <div className="settings-row-info">
        <h3 className="settings-row-title">{t("settings.micTest.title")}</h3>
        <p className="settings-row-description">{micTestDescription(state, t)}</p>
        {error ? <p className="settings-row-error">{error}</p> : null}
      </div>
      <div className="settings-row-control settings-mic-test-control">
        {state === "recording" ? (
          <>
            <div className="settings-mic-test-meter">
              <div
                className="settings-mic-test-meter-fill"
                role="progressbar"
                aria-label={t("settings.micTest.levelAria")}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(level * 100)}
                style={{
                  transform: `scaleX(${Math.max(0, Math.min(1, level))})`,
                }}
              />
            </div>
            <MicTestTime startedAt={startedAt} durationSeconds={durationSeconds} />
          </>
        ) : null}
        {sampleSrc && state === "ready" ? (
          <MicTestPlayer
            src={sampleSrc}
            playing={playing}
            durationSeconds={durationSeconds}
            onPlaybackError={onPlaybackError}
            onPlayingChange={onPlayingChange}
          />
        ) : null}
        {state === "ready" ? (
          <button
            type="button"
            className="btn btn-secondary"
            disabled={playing}
            onClick={onStartOver}
          >
            {t("settings.micTest.startOver")}
          </button>
        ) : null}
        {state !== "recording" && state !== "ready" ? (
          <button type="button" className="btn btn-secondary" onClick={onStart}>
            {t("settings.micTest.start")}
          </button>
        ) : null}
      </div>
    </div>
  );
}

function micTestDescription(state: MicTestState, t: TFunction) {
  if (state === "recording") {
    return t("settings.micTest.recording");
  }
  if (state === "ready") {
    return t("settings.micTest.ready");
  }
  return t("settings.micTest.idle");
}

function formatMicTestTime(milliseconds: number, durationSeconds: number) {
  const seconds = Math.min(durationSeconds, Math.floor(Math.max(0, milliseconds) / 1000));
  return formatMicTestDuration(seconds);
}

function formatMicTestDuration(seconds: number) {
  return `00:${seconds.toString().padStart(2, "0")}`;
}

function MicTestPlayer({
  src,
  playing,
  durationSeconds,
  onPlaybackError,
  onPlayingChange,
}: {
  src: string;
  playing: boolean;
  durationSeconds: number;
  onPlaybackError: () => void;
  onPlayingChange: (playing: boolean) => void;
}) {
  const t = useT();
  const audioRef = useRef<HTMLAudioElement>(null);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(durationSeconds);

  async function playSample() {
    const audio = audioRef.current;
    if (!audio) return;
    try {
      audio.currentTime = 0;
      setCurrentTime(0);
      await audio.play();
      onPlayingChange(true);
    } catch {
      onPlayingChange(false);
      onPlaybackError();
    }
  }

  function seek(nextValue: string) {
    const next = Number(nextValue);
    if (!Number.isFinite(next)) return;
    setCurrentTime(next);
    if (audioRef.current) {
      audioRef.current.currentTime = next;
    }
  }

  return (
    <>
      <audio
        ref={audioRef}
        className="settings-mic-test-hidden-audio"
        src={src}
        preload="metadata"
        onLoadedMetadata={(event) => {
          const nextDuration = event.currentTarget.duration;
          if (Number.isFinite(nextDuration) && nextDuration > 0) {
            setDuration(nextDuration);
          }
        }}
        onTimeUpdate={(event) => setCurrentTime(event.currentTarget.currentTime)}
        onPause={() => onPlayingChange(false)}
        onPlay={() => onPlayingChange(true)}
        onEnded={() => {
          onPlayingChange(false);
          setCurrentTime(0);
          if (audioRef.current) audioRef.current.currentTime = 0;
        }}
        onError={onPlaybackError}
      />
      {playing ? (
        <input
          className="settings-mic-test-mini-scrubber"
          type="range"
          min={0}
          max={Math.max(duration, durationSeconds)}
          step={0.1}
          value={Math.min(currentTime, duration)}
          aria-label={t("settings.micTest.playbackAria")}
          onChange={(event) => seek(event.currentTarget.value)}
        />
      ) : (
        <button
          type="button"
          className="btn btn-secondary"
          aria-label={t("settings.micTest.playAria")}
          onClick={() => void playSample()}
        >
          {t("settings.micTest.play")}
        </button>
      )}
    </>
  );
}
