import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef } from "react";
import { recordingToStatus } from "./app-helpers";
import { invalidateNoteTabs } from "./tabs/tabs";
import {
  finishRecording,
  getNote,
  listNotes,
  pauseRecording,
  resolveAgentRecorderRequest,
  resumeRecording,
  recoverRecording,
  retryProcessing,
  validateAgentRecorderRequest,
} from "../lib/tauri";
import { playRecordingSound } from "../lib/recording-sounds";
import { AGENT_RECORDER_REQUEST_EVENT } from "../lib/events";
import { errorCode, messageFromError } from "../lib/errors";
import { getCurrentDataPartitionName } from "../lib/data-partition";
import type { RecordingSourceMode, RecordingStatusDto } from "../lib/tauri";
import { RECORD_NOTICES_DEMO_SESSION_ID } from "./processing-demo-ids";
import type { AgentRecorderRequestPayload } from "./app-shell";
import type { UseRecordingControlsDependencies } from "./use-recording-controls-types";

export function useRecordingControls(dependencies: UseRecordingControlsDependencies) {
  const {
    activeViewRef,
    appBlocked,
    bootstrapped,
    crossPartitionRecordingNoteIdRef,
    dispatch,
    finishingSessionsRef,
    handleStartAgentRecording,
    recordNoticesDemoRef,
    recordingNoteIdRef,
    recordingStatusRef,
    selectedNote,
    setActiveView,
    setError,
    setFolderReturnTarget,
    setOriginAllNotes,
    setOriginFolderId,
    setRecordingNote,
    setTabs,
    state,
    tabsRef,
  } = dependencies;

  const agentRecorderHandlerRef = useRef<(payload: AgentRecorderRequestPayload) => Promise<void>>(
    async () => {},
  );
  agentRecorderHandlerRef.current = async (payload: AgentRecorderRequestPayload) => {
    const requestId = typeof payload.requestId === "string" ? payload.requestId : "";
    if (!requestId) return;

    const resolve = (result: {
      ok: boolean;
      noteId?: string;
      noteTitle?: string;
      sessionId?: string;
      status?: RecordingStatusDto;
      errorCode?: string;
      errorMessage?: string;
      errorDetails?: unknown;
    }) => {
      void resolveAgentRecorderRequest({ requestId, ...result }).catch(async (err) => {
        console.warn("Failed to resolve agent recorder request", err);
        // A transient resolve failure (IPC hiccup, poisoned lock) happens
        // while the proxy is still waiting inside its lease: retry once
        // instead of treating it as an expired request.
        if (errorCode(err) !== "agent_recorder_request_not_found") {
          try {
            await resolveAgentRecorderRequest({ requestId, ...result });
            return;
          } catch (retryErr) {
            if (errorCode(retryErr) !== "agent_recorder_request_not_found") {
              console.warn("Agent recorder resolve retry failed", retryErr);
              return;
            }
          }
        }
        // Native admission already checked this request at the hardware boundary.
        // A successful start belongs to the visible recorder once published;
        // ending the chat or losing a late ACK does not stop that meeting.
      });
    };

    if (appBlocked || !bootstrapped) {
      resolve({
        ok: false,
        errorCode: "app_not_ready",
        errorMessage: "Clovy is not ready to start or stop recording yet.",
      });
      return;
    }

    try {
      const targetProfile = typeof payload.profile === "string" ? payload.profile : undefined;
      const targetNoteId = typeof payload.noteId === "string" ? payload.noteId : undefined;
      const targetSessionId = typeof payload.sessionId === "string" ? payload.sessionId : undefined;
      if (targetProfile) await validateAgentRecorderRequest(requestId);
      if (payload.action === "recover" || payload.action === "retry") {
        if (!targetSessionId || !targetNoteId || !targetProfile)
          throw new Error("The saved recording target is missing.");
        if (
          payload.action === "recover" &&
          payload.recoveryAction !== "validate" &&
          payload.recoveryAction !== "discard"
        )
          throw new Error("Recovery action must be validate or discard.");
        const note =
          payload.action === "recover"
            ? await recoverRecording(
                targetSessionId,
                payload.recoveryAction as "validate" | "discard",
                targetProfile,
                requestId,
              )
            : await retryProcessing(targetNoteId, targetSessionId, targetProfile, requestId);
        if (note.id !== targetNoteId)
          throw new Error("The processing response does not match the authorized note.");
        if (getCurrentDataPartitionName() === targetProfile)
          dispatch({ type: "noteProcessingUpdated", note });
        resolve({ ok: true, noteId: note.id, noteTitle: note.title, sessionId: targetSessionId });
        return;
      }
      if (payload.action === "pause" || payload.action === "resume") {
        const active = recordingStatusRef.current;
        if (
          !active ||
          !targetSessionId ||
          active.sessionId !== targetSessionId ||
          active.noteId !== targetNoteId
        ) {
          resolve({
            ok: false,
            errorCode: "recording_target_mismatch",
            errorMessage: "The authorized recording is no longer active.",
          });
          return;
        }
        const status =
          payload.action === "pause"
            ? await pauseRecording(targetSessionId, targetProfile, requestId)
            : await resumeRecording(targetSessionId, targetProfile, requestId);
        if (status.sessionId !== targetSessionId || status.noteId !== targetNoteId)
          throw new Error("The recorder response has a different target.");
        recordingStatusRef.current = status;
        dispatch({ type: "recordingStatusChanged", status });
        resolve({ ok: true, noteId: targetNoteId, sessionId: targetSessionId, status });
        return;
      }
      if (payload.action === "start") {
        const requestedSourceMode: RecordingSourceMode =
          payload.sourceMode === "microphonePlusSystem" ? "microphonePlusSystem" : "microphoneOnly";
        const note =
          targetProfile && targetNoteId
            ? await handleStartAgentRecording(requestedSourceMode, {
                noteId: targetNoteId,
                profile: targetProfile,
                requestId,
              })
            : await handleStartAgentRecording(requestedSourceMode);
        const status = recordingStatusRef.current;
        resolve({
          ok: true,
          noteId: note.id,
          noteTitle: note.title,
          ...(targetProfile ? { sessionId: status?.sessionId, status } : {}),
        });
        return;
      }
      if (payload.action === "stop") {
        const activeRecording = recordingStatusRef.current;
        const noteId = recordingNoteIdRef.current ?? activeRecording?.noteId;
        const noteTitle = noteId
          ? (state.notes.find((note) => note.id === noteId)?.title ?? selectedNote?.title)
          : undefined;
        if (!activeRecording) {
          resolve({
            ok: false,
            errorCode: "recording_not_found",
            errorMessage: "No recording is currently running.",
          });
          return;
        }
        if (
          (targetSessionId && activeRecording.sessionId !== targetSessionId) ||
          (targetNoteId && noteId !== targetNoteId)
        ) {
          resolve({
            ok: false,
            errorCode: "recording_target_mismatch",
            errorMessage: "The authorized recording is no longer active.",
          });
          return;
        }
        const finished = await handleFinishRecording(activeRecording.sessionId, {
          rethrow: true,
          expectedProfile: targetProfile,
          agentRequestId: targetProfile ? requestId : undefined,
        });
        if (
          !finished ||
          finished.recording.id !== activeRecording.sessionId ||
          finished.note.id !== noteId
        )
          throw new Error(
            "The recording finalization did not confirm its authorized session and note.",
          );
        const finalStatus = recordingToStatus(finished.recording);
        resolve({
          ok: true,
          noteId,
          noteTitle,
          ...(targetSessionId ? { sessionId: targetSessionId, status: finalStatus } : {}),
        });
        return;
      }
      resolve({
        ok: false,
        errorCode: "invalid_action",
        errorMessage: "Unknown recorder action.",
      });
    } catch (err) {
      resolve({
        ok: false,
        errorCode: errorCode(err) ?? "agent_recorder_failed",
        errorMessage: messageFromError(err),
        ...(err && typeof err === "object" && "details" in err
          ? { errorDetails: err.details }
          : {}),
      });
    }
  };

  useEffect(() => {
    let aborted = false;
    let unlisten: (() => void) | undefined;
    void listen(AGENT_RECORDER_REQUEST_EVENT, (event) => {
      void agentRecorderHandlerRef.current((event.payload ?? {}) as AgentRecorderRequestPayload);
    }).then((cleanup) => {
      if (aborted) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      aborted = true;
      unlisten?.();
    };
  }, []);

  async function handleFinishRecording(
    sessionId: string,
    options: { rethrow?: boolean; expectedProfile?: string; agentRequestId?: string } = {},
  ) {
    // The dev __recordNoticesDemo session has no backend recording — stopping it
    // just tears the demo down (clears the synthetic status and pins) instead of
    // calling finishRecording, which would fail with "recording not found".
    if (import.meta.env.DEV && sessionId === RECORD_NOTICES_DEMO_SESSION_ID) {
      recordNoticesDemoRef.current?.clear();
      return;
    }
    // The recorder bar stays mounted (and clickable) for the duration of its
    // exit animation after the first stop click, so a fast double-click would
    // fire finishRecording twice — the second call fails with a scary
    // "recording not found" error. Gate per session until the call settles.
    if (finishingSessionsRef.current.has(sessionId)) return;
    finishingSessionsRef.current.add(sessionId);
    // Collapse the shell back to idle the instant stop is pressed so it
    // never lingers wide while the (potentially long) transcribe +
    // generate pipeline runs. Processing is queued per note, so the record
    // button stays available — you can stack another take while this one
    // finishes — and the body shimmer ("Transcribing audio…" → "Generating
    // notes…") plus a queued count tell the user work is still in flight.
    const owningNoteId = recordingNoteIdRef.current;
    const wasCrossPartitionRecording =
      !!owningNoteId && crossPartitionRecordingNoteIdRef.current === owningNoteId;
    dispatch({ type: "recordingStatusCleared" });
    setRecordingNote(undefined);
    if (wasCrossPartitionRecording && owningNoteId) {
      crossPartitionRecordingNoteIdRef.current = undefined;
      const nextTabs = invalidateNoteTabs(tabsRef.current, new Set([owningNoteId]));
      if (nextTabs !== tabsRef.current) {
        tabsRef.current = nextTabs;
        setTabs(nextTabs);
      }
      // The previous partition's note was temporarily present only to control the
      // recording. Once the take stops, remove it from the current data partition's
      // visible list before any tab or sidebar action can reopen it.
      dispatch({
        type: "notesLoaded",
        notes: state.notes.filter((note) => note.id !== owningNoteId),
      });
      setOriginFolderId(undefined);
      setOriginAllNotes(false);
      setFolderReturnTarget(undefined);
      if (activeViewRef.current === "meetings") setActiveView("notes");
    }
    playRecordingSound("stop");
    // Optimistically flip the note that owns this recording to transcribing.
    // The selected note isn't necessarily that note — the user may have
    // browsed elsewhere while recording — and stamping the wrong note as
    // transcribing would lock its record button and shimmer forever.
    if (selectedNote && selectedNote.id === owningNoteId) {
      dispatch({
        type: "noteProcessingUpdated",
        note: {
          ...selectedNote,
          processingStatus: "transcribing",
          lastError:
            selectedNote.processingStatus === "failed" ||
            selectedNote.processingStatus === "recoverable"
              ? undefined
              : selectedNote.lastError,
        },
      });
    }
    try {
      const result = options.expectedProfile
        ? await finishRecording(sessionId, options.expectedProfile, options.agentRequestId)
        : await finishRecording(sessionId);
      // The result belongs to the partition where recording started. Once that
      // partition's temporary recording view has been retired, do not let the
      // finish response upsert the old note into the newly selected data partition.
      if (!wasCrossPartitionRecording) {
        dispatch({ type: "noteProcessingUpdated", note: result.note });
      }
      return result;
    } catch (err) {
      if (
        wasCrossPartitionRecording ||
        !owningNoteId ||
        !(await applyNoteScopedProcessingFailure(owningNoteId, err))
      ) {
        setError(messageFromError(err));
      }
      if (options.rethrow) throw err;
    } finally {
      if (wasCrossPartitionRecording) {
        const finishingPartition = getCurrentDataPartitionName();
        try {
          const response = await listNotes();
          if (getCurrentDataPartitionName() === finishingPartition) {
            dispatch({ type: "notesLoaded", notes: response.items });
          }
        } catch (refreshErr) {
          if (getCurrentDataPartitionName() === finishingPartition) {
            setError(messageFromError(refreshErr));
          }
        }
      }
      finishingSessionsRef.current.delete(sessionId);
    }
  }

  async function applyNoteScopedProcessingFailure(noteId: string, err: unknown) {
    try {
      const note = await getNote(noteId);
      if (note.processingStatus !== "failed") return false;
      dispatch({ type: "noteProcessingUpdated", note });
      setError(null);
      return true;
    } catch {
      return false;
    }
  }

  const handlePauseRecording = useCallback(async (sessionId: string) => {
    // The dev __recordNoticesDemo session has no backend recording; report
    // success without a pauseRecording IPC round-trip. Its own ticker keeps the
    // bar live, so pause is a visual no-op here.
    if (import.meta.env.DEV && sessionId === RECORD_NOTICES_DEMO_SESSION_ID) {
      return true;
    }
    try {
      const status = await pauseRecording(sessionId);
      dispatch({ type: "recordingStatusChanged", status });
      playRecordingSound("pause");
      return true;
    } catch (err) {
      setError(messageFromError(err));
      return false;
    }
  }, []);

  async function handleResumeRecording(sessionId: string) {
    // The dev __recordNoticesDemo session has no backend recording; its ticker
    // already keeps the bar in the recording state, so resume is a no-op.
    if (import.meta.env.DEV && sessionId === RECORD_NOTICES_DEMO_SESSION_ID) {
      return;
    }
    playRecordingSound("start");
    try {
      const status = await resumeRecording(sessionId);
      dispatch({ type: "recordingStatusChanged", status });
    } catch (err) {
      setError(messageFromError(err));
    }
  }

  return {
    applyNoteScopedProcessingFailure,
    handleFinishRecording,
    handlePauseRecording,
    handleResumeRecording,
  };
}
