import { type FormEvent, useEffect, useId, useLayoutEffect, useState } from "react";
import { type MessageKey, useT } from "../../i18n";
import { errorCode } from "../../lib/errors";
import { subscribeRecordingMarks } from "../../lib/recording-marks";
import {
  addRecordingMark,
  listRecordingMarks,
  type RecordingAssistanceStatusDto,
  type RecordingMarkDto,
  type RecordingStatusDto,
  suggestRecordingQuestion,
} from "../../lib/tauri";
import { formatElapsed } from "../recorder/RecorderBar";
import { InlineNotice } from "../ui/InlineNotice";
import { Spinner } from "../ui/Spinner";

type RecordingAssistProps = {
  noteId: string;
  recording?: RecordingStatusDto;
  assistance?: RecordingAssistanceStatusDto;
  assistanceStatusAvailable: boolean;
};

function mergeMarks(existing: RecordingMarkDto[], incoming: RecordingMarkDto[]) {
  const marks = new Map(existing.map((mark) => [mark.id, mark]));
  for (const mark of incoming) marks.set(mark.id, mark);
  return [...marks.values()].sort(
    (left, right) =>
      left.createdAt.localeCompare(right.createdAt) || left.elapsedMs - right.elapsedMs,
  );
}

export function RecordingAssist({
  noteId,
  recording,
  assistance,
  assistanceStatusAvailable,
}: RecordingAssistProps) {
  const t = useT();
  const [marks, setMarks] = useState<RecordingMarkDto[]>([]);
  const [loadError, setLoadError] = useState(false);
  const [loadAttempt, setLoadAttempt] = useState(0);

  useLayoutEffect(
    () =>
      subscribeRecordingMarks((mark) => {
        if (mark.noteId === noteId) setMarks((marks) => mergeMarks(marks, [mark]));
      }),
    [noteId],
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies(loadAttempt): explicit retry refreshes the saved marks.
  useEffect(() => {
    let current = true;
    void listRecordingMarks(noteId)
      .then((saved) => {
        if (!current) return;
        setMarks((marks) =>
          mergeMarks(
            marks,
            saved.filter((mark) => mark.noteId === noteId),
          ),
        );
        setLoadError(false);
      })
      .catch(() => {
        if (current) setLoadError(true);
      });
    return () => {
      current = false;
    };
  }, [noteId, loadAttempt]);

  const active = recording?.state === "recording" || recording?.state === "paused";
  const visibleMarks = active
    ? marks.filter((mark) => mark.sessionId === recording.sessionId)
    : marks;

  if (!active && !visibleMarks.length && !loadError) return null;

  return (
    <section className="recording-assist" aria-label={t("notes.assist.region")}>
      {active ? (
        <div key={recording.sessionId} className="recording-assist-actions">
          <RecordingMarkForm noteId={noteId} sessionId={recording.sessionId} />
          <RecordingQuestionControl
            noteId={noteId}
            recording={recording}
            assistance={assistance}
            assistanceStatusAvailable={assistanceStatusAvailable}
          />
        </div>
      ) : null}
      {visibleMarks.length ? (
        <ul className="recording-assist-marks" aria-label={t("notes.assist.marks")}>
          {visibleMarks.map((mark) => (
            <li key={mark.id}>
              <time dateTime={`PT${Math.floor(mark.elapsedMs / 1000)}S`}>
                {formatElapsed(mark.elapsedMs)}
              </time>
              <span>{mark.label || t("notes.assist.mark")}</span>
            </li>
          ))}
        </ul>
      ) : null}
      {loadError ? (
        <InlineNotice
          role="alert"
          body={t("notes.assist.loadError")}
          actions={
            <button
              className="primary-action"
              type="button"
              onClick={() => {
                setLoadError(false);
                setLoadAttempt((attempt) => attempt + 1);
              }}
            >
              {t("notes.assist.retry")}
            </button>
          }
        />
      ) : null}
    </section>
  );
}

function RecordingMarkForm({ noteId, sessionId }: { noteId: string; sessionId: string }) {
  const t = useT();
  const [label, setLabel] = useState("");
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);

  async function addMark(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (saving) return;
    setFailed(false);
    setSaving(true);
    try {
      const mark = await addRecordingMark(noteId, sessionId, label.trim() || undefined);
      if (mark.noteId !== noteId || mark.sessionId !== sessionId) return;
      setLabel("");
    } catch {
      setFailed(true);
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="recording-assist-group">
      <form className="recording-assist-mark-form" onSubmit={addMark}>
        <input
          className="dialog-input"
          aria-label={t("notes.assist.label")}
          maxLength={120}
          placeholder={t("notes.assist.label")}
          value={label}
          disabled={saving}
          onChange={(event) => setLabel(event.currentTarget.value)}
        />
        <button className="primary-action" type="submit" disabled={saving}>
          {saving ? <Spinner aria-hidden /> : null}
          {t("notes.assist.addMark")}
        </button>
      </form>
      {failed ? <InlineNotice role="alert" body={t("notes.assist.saveError")} /> : null}
    </div>
  );
}

function RecordingQuestionControl({
  noteId,
  recording,
  assistance,
  assistanceStatusAvailable,
}: {
  noteId: string;
  recording: RecordingStatusDto;
  assistance?: RecordingAssistanceStatusDto;
  assistanceStatusAvailable: boolean;
}) {
  const t = useT();

  const unavailableReason =
    recording.state === "paused"
      ? t("notes.assist.paused")
      : !recording.livePreviewEnabled || assistance?.enabled === false
        ? t("notes.assist.previewOff")
        : undefined;

  if (!unavailableReason && assistance?.enabled) {
    return (
      <RecordingQuestion
        key={assistance.epoch}
        noteId={noteId}
        sessionId={recording.sessionId}
        hasRecentContext={assistance.hasRecentContext}
        assistanceStatusAvailable={assistanceStatusAvailable}
      />
    );
  }

  return (
    <div className="recording-assist-group">
      <button className="primary-action" type="button" disabled>
        {t("notes.assist.suggest")}
      </button>
      {unavailableReason ? <p className="recording-assist-hint">{unavailableReason}</p> : null}
    </div>
  );
}

function RecordingQuestion({
  noteId,
  sessionId,
  hasRecentContext,
  assistanceStatusAvailable,
}: {
  noteId: string;
  sessionId: string;
  hasRecentContext: boolean;
  assistanceStatusAvailable: boolean;
}) {
  const t = useT();
  const hintId = useId();
  const [pending, setPending] = useState(false);
  const [question, setQuestion] = useState("");
  const [failure, setFailure] = useState<MessageKey>();
  const requestDisabled = pending || !assistanceStatusAvailable || !hasRecentContext;
  const hint = !assistanceStatusAvailable
    ? "notes.assist.statusUnavailable"
    : hasRecentContext
      ? "notes.assist.engineHint"
      : "notes.assist.waiting";

  async function suggest() {
    if (requestDisabled) return;
    setPending(true);
    setFailure(undefined);
    setQuestion("");
    try {
      setQuestion(await suggestRecordingQuestion(noteId, sessionId));
    } catch (error) {
      setFailure(suggestionErrors.get(errorCode(error) ?? "") ?? "notes.assist.suggestError");
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="recording-assist-group">
      <button
        className="primary-action"
        type="button"
        aria-describedby={hintId}
        disabled={requestDisabled}
        onClick={suggest}
      >
        {pending ? <Spinner aria-hidden /> : null}
        {pending ? t("notes.assist.suggesting") : t("notes.assist.suggest")}
      </button>
      <p id={hintId} className="recording-assist-hint">
        {t(hint)}
      </p>
      {failure ? <InlineNotice role="alert" body={t(failure)} /> : null}
      {question ? (
        <p className="recording-assist-question" role="status">
          {question}
        </p>
      ) : null}
    </div>
  );
}

const suggestionErrors = new Map<string, MessageKey>([
  ["recording_suggestion_provider_missing", "notes.assist.engineRequired"],
  ["recording_suggestion_tools_unavailable", "notes.assist.toolsUnavailable"],
  ["recording_suggestion_cooldown", "notes.assist.cooldown"],
  ["recording_suggestion_busy", "notes.assist.cooldown"],
  ["recording_suggestion_no_context", "notes.assist.waiting"],
  ["recording_suggestion_inactive", "notes.assist.inactive"],
]);
