import { useEffect, useRef, useState } from "react";
import { useT } from "../../i18n";
import { messageFromError } from "../../lib/errors";
import {
  type LocalTranscriptionModelState,
  localTranscriptionModelState,
  installLocalTranscriptionModel,
  cancelLocalTranscriptionModelInstall,
  removeLocalTranscriptionModel,
  onLocalTranscriptionModelState,
} from "../../lib/tauri";
import "./local-transcription-settings.css";

export function LocalTranscriptionSettings({
  supported,
  provider,
  model,
  onSelect,
  onRemoved,
  readOnly = false,
}: {
  supported: boolean;
  provider: string;
  model?: string;
  onSelect: (model: string) => Promise<void>;
  onRemoved: (result?: { switched_to_groq: boolean }) => Promise<void>;
  readOnly?: boolean;
}) {
  const t = useT();
  // D6 stores only the active model. Remember the Groq choice while this settings view is open.
  const lastGroqModel = useRef("whisper-large-v3-turbo");
  useEffect(() => {
    if (
      provider === "groq" &&
      (model === "whisper-large-v3" || model === "whisper-large-v3-turbo")
    ) {
      lastGroqModel.current = model;
    }
  }, [provider, model]);
  const [state, setState] = useState<LocalTranscriptionModelState>();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  useEffect(() => {
    if (!supported) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let revision = 0;
    const receive = (next: LocalTranscriptionModelState) => {
      revision += 1;
      if (!disposed) setState(next);
    };
    void (async () => {
      try {
        const stop = await onLocalTranscriptionModelState(receive);
        if (disposed) {
          stop();
          return;
        }
        unlisten = stop;
        const snapshotRevision = revision;
        const snapshot = await localTranscriptionModelState();
        if (!disposed && revision === snapshotRevision) setState(snapshot);
      } catch (error) {
        if (!disposed) setMessage(messageFromError(error));
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [supported]);
  if (!supported) return null;
  const size = (bytes: number) =>
    `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 }).format(bytes / 1_000_000)} MB`;
  async function action(run: () => Promise<void>) {
    if (busy) return;
    setBusy(true);
    setMessage("");
    try {
      await run();
    } catch (error) {
      setMessage(messageFromError(error));
    } finally {
      setBusy(false);
    }
  }
  function install() {
    // Installation remains cancellable while its command is streaming.
    setMessage("");
    setState({ state: "downloading", received: 0, total: 632162964 });
    void installLocalTranscriptionModel(setState).catch(async (error) => {
      try {
        const snapshot = await localTranscriptionModelState();
        setState(snapshot);
        // The failed state already renders a localized recovery message.
        if (snapshot.state !== "failed") setMessage(messageFromError(error));
      } catch {
        setMessage(messageFromError(error));
      }
    });
  }
  return (
    <div className="local-transcription-settings">
      <fieldset className="local-transcription-engines" disabled={readOnly || busy}>
        <legend className="sr-only">{t("settings.localTranscription.engine")}</legend>
        <label
          className="local-transcription-choice"
          data-selected={provider === "groq"}
          data-disabled={busy || readOnly}
        >
          <input
            type="radio"
            name="transcription-engine"
            checked={provider === "groq"}
            onChange={() => void action(() => onSelect(lastGroqModel.current))}
          />
          <span>
            <span className="local-transcription-name">
              {t("settings.localTranscription.groq")}
            </span>
            <span className="settings-row-description">
              {t("settings.localTranscription.groqDescription")}
            </span>
          </span>
        </label>
        <label
          className="local-transcription-choice"
          data-selected={provider === "local"}
          data-disabled={busy || readOnly || state?.state !== "ready"}
        >
          <input
            type="radio"
            name="transcription-engine"
            checked={provider === "local"}
            disabled={state?.state !== "ready"}
            onChange={() => void action(() => onSelect("parakeet-ultra"))}
            aria-describedby="local-transcription-availability"
          />
          <span>
            <span className="local-transcription-name">
              {t("settings.localTranscription.local")}
            </span>
            <span className="settings-row-description">
              {t("settings.localTranscription.localDescription")}
            </span>
          </span>
        </label>
      </fieldset>
      <div className="local-transcription-download">
        <div
          id="local-transcription-availability"
          className="settings-row-description"
          role="status"
        >
          {!state
            ? t("settings.localTranscription.loading")
            : state.state === "ready"
              ? t("settings.localTranscription.ready", { size: size(state.bytes) })
              : state.state === "downloading"
                ? `${size(state.received)} / ${size(state.total)}`
                : t("settings.localTranscription.required")}
        </div>
        {state?.state === "downloading" && (
          <progress
            aria-label={t("settings.localTranscription.progress")}
            max={state.total}
            value={state.received}
          />
        )}
        {state?.state === "failed" && (
          <p className="settings-row-description" role="alert">
            {state.reason.startsWith("network: ")
              ? t("settings.localTranscription.networkFailed", { file: state.reason.slice(9) })
              : t("settings.localTranscription.failed", { reason: state.reason })}
          </p>
        )}
        <div>
          {(state?.state === "absent" || state?.state === "failed") && (
            <button
              type="button"
              className="btn btn-secondary"
              disabled={busy || readOnly}
              onClick={install}
            >
              {t(
                state.state === "failed"
                  ? "settings.localTranscription.retry"
                  : "settings.localTranscription.install",
              )}
            </button>
          )}
          {state?.state === "downloading" && (
            <button
              type="button"
              className="btn btn-secondary"
              disabled={busy || readOnly}
              onClick={() => void action(cancelLocalTranscriptionModelInstall)}
            >
              {t("settings.localTranscription.cancel")}
            </button>
          )}
          {state?.state === "ready" && (
            <button
              type="button"
              className="btn btn-secondary"
              disabled={busy || readOnly}
              onClick={() =>
                void action(async () => {
                  // The backend switches engines before filesystem removal, which can fail.
                  let result: { switched_to_groq: boolean };
                  try {
                    result = await removeLocalTranscriptionModel();
                  } catch (error) {
                    await onRemoved().catch(() => undefined);
                    const snapshot = await localTranscriptionModelState().catch(() => ({
                      state: "absent" as const,
                    }));
                    setState(snapshot);
                    throw error;
                  }
                  setState({ state: "absent" });
                  if (result.switched_to_groq) setMessage(t("settings.localTranscription.removed"));
                  // The command's committed result remains authoritative if the refresh fails.
                  await onRemoved(result).catch(() => undefined);
                })
              }
            >
              {t("settings.localTranscription.remove")}
            </button>
          )}
        </div>
        {message && (
          <p className="settings-row-description" role="status">
            {message}
          </p>
        )}
      </div>
    </div>
  );
}
