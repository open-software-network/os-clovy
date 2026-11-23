import { IconArrowRotateClockwise } from "central-icons/IconArrowRotateClockwise";
import { useState } from "react";
import { type MessageKey, t as translate, useT } from "../../i18n";

type Props = {
  errorMessage?: string;
  audioPreserved: boolean;
  onRetry: () => void | Promise<void>;
};

export function userFacingFailureMessage(message?: string) {
  if (!message) return undefined;
  return message
    .split("|")
    .map((part) => friendlyFailureSegment(part.trim()))
    .filter(Boolean)
    .join(" | ");
}

/** Failure sentences the desktop backend writes into a note
 * (`domain/processing.rs`: `user_facing_transcription_failure_message`,
 * `note_generation_failure_for_user`, and direct providers for a missing provider
 * key or oversized audio). Notes persist only that English
 * sentence, not its error code, so the sentence is the stable identifier:
 * each key's English text is byte-identical to what Rust stores, and the
 * banner shows it in the interface language. */
const STORED_FAILURE_KEYS: MessageKey[] = [
  "notes.failure.transcriptionEngineChanged",
  "notes.failure.localTranscriberFailed",
  "notes.failure.noSpeech",
  "notes.failure.silenceSystem",
  "notes.failure.silenceMicrophone",
  "notes.failure.providerFailed",
  "notes.failure.serviceBusy",
  "notes.failure.generationBusy",
  "notes.failure.generationFailed",
  "notes.failure.groqKeyMissing",
  "notes.failure.audioTooLarge",
  "notes.failure.groqUsageLimit",
  "notes.failure.groqKeyUnavailable",
  "notes.failure.generationKeyMissing",
];

/** `notes.failure.rateLimitPaused` as Rust stores it
 * (`processing.rs`: `rate_limit_paused_message`), with the HH:MM it resumes. */
const RATE_LIMIT_PAUSED =
  /^Groq usage limit reached\. Clovy continues transcribing at (\d{1,2}:\d{2})\. Parts already transcribed are kept\.$/;

function storedFailureKey(body: string): MessageKey | undefined {
  const trimmed = body.trim();
  return STORED_FAILURE_KEYS.find((key) => translate(key, undefined, "en") === trimmed);
}

function friendlyFailureSegment(message: string) {
  const source = message.match(/^(Microphone|System):\s*/i)?.[1];
  const body = source ? message.replace(/^(Microphone|System):\s*/i, "") : message;
  const normalized = body.toLowerCase();
  let friendly = body;
  const storedKey = storedFailureKey(body);
  const pausedUntil = body.trim().match(RATE_LIMIT_PAUSED)?.[1];
  if (storedKey) {
    friendly = translate(storedKey);
  } else if (pausedUntil) {
    friendly = translate("notes.failure.rateLimitPaused", { time: pausedUntil });
  } else if (normalized.includes("no_speech") || normalized.includes("no speech")) {
    friendly = translate("notes.failure.noSpeech");
  } else if (isInvalidClovyResponseMessage(body)) {
    friendly = translate("notes.failure.invalidResponse");
  } else if (
    normalized.includes("metering_provider_failed") ||
    normalized.startsWith("billing is temporarily unavailable")
  ) {
    friendly = translate("notes.failure.providerFailed");
  } else if (normalized.includes("upstream_provider_failed")) {
    friendly = translate("notes.failure.providerFailed");
  } else if (normalized.includes("authorization_denied")) {
    friendly = translate("notes.failure.serviceBusy");
  }
  return source
    ? translate("notes.failure.sourcePrefixed", {
        source: failureSourceLabel(source),
        message: friendly,
      })
    : friendly;
}

/** The transcript source a backend failure names ("Microphone: ..."), shown in
 * the interface language; any other casing passes through untouched. */
function failureSourceLabel(source: string) {
  if (source === "Microphone") return translate("notes.editor.source.microphone");
  if (source === "System") return translate("notes.editor.source.system");
  return source;
}

export function isInvalidClovyResponseMessage(message: string) {
  const normalized = message.trim().toLowerCase();
  return (
    normalized.includes("clovy_api_response_invalid") ||
    normalized.includes("processing service returned an invalid response") ||
    /^expected value at line \d+ column \d+$/.test(normalized)
  );
}

export function NoteFailureBanner({ errorMessage, audioPreserved, onRetry }: Props) {
  const t = useT();
  const displayMessage = userFacingFailureMessage(errorMessage);
  // Local busy flag so a fast double-click can't fire onRetry twice. The
  // banner unmounts when the note transitions out of `failed` status, so we
  // don't need to reset this state ourselves; the catch covers the case
  // where onRetry rejects and the note stays in `failed`.
  const [retrying, setRetrying] = useState(false);
  // Each retry click advances the
  // rotation by a full turn so the arrow sweeps once on press.
  const [spins, setSpins] = useState(0);

  async function handleRetry() {
    if (retrying) return;
    setRetrying(true);
    setSpins((turns) => turns + 1);
    try {
      await onRetry();
    } catch {
      // Parent already surfaces errors; release the gate so the user can try
      // again rather than getting stuck in a frozen spinner.
      setRetrying(false);
    }
  }

  return (
    <aside className="note-failure-banner" role="alert">
      <p className="note-failure-message">
        {displayMessage ?? t("notes.failure.generic")}
        {audioPreserved ? ` ${t("notes.failure.savedCanRetry")}` : null}
      </p>
      <div className="note-failure-actions">
        <button
          type="button"
          className="btn btn-ghost"
          onClick={() => void handleRetry()}
          disabled={!audioPreserved || retrying}
          aria-busy={retrying || undefined}
        >
          <IconArrowRotateClockwise
            size={14}
            className="balance-refresh-icon"
            style={{ transform: `rotate(${spins * 360}deg)` }}
          />
          {t("common.retry")}
        </button>
      </div>
    </aside>
  );
}
