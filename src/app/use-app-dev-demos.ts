import { useEffect, useRef, useState } from "react";
import type { GlobalRecorderDemoApi } from "../lib/global-recorder-demo";
import type { RecordNoticesDemoApi } from "../lib/record-notices-demo";
import type { RecordingStatusDto } from "../lib/tauri";
import { RECORD_NOTICES_DEMO_SESSION_ID } from "./processing-demo-ids";
import type { UseAppDevDemosDependencies } from "./use-app-dev-demos-types";

export function useAppDevDemos(dependencies: UseAppDevDemosDependencies) {
  const {
    dispatch,
    getSelectedNoteId,
    recordingStatusRef,
    setActiveView,
    setLiveTranscriptEvents,
    setRecordingNote,
  } = dependencies;

  const [demoRecorderStatus, setDemoRecorderStatus] = useState<RecordingStatusDto | null>(null);
  const demoRecorderRef = useRef<GlobalRecorderDemoApi | null>(null);
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    let cancelled = false;
    void import("../lib/global-recorder-demo").then(({ registerGlobalRecorderDemo }) => {
      if (cancelled) return;
      demoRecorderRef.current = registerGlobalRecorderDemo({
        setStatus: setDemoRecorderStatus,
      });
    });
    return () => {
      cancelled = true;
      demoRecorderRef.current?.dispose();
      demoRecorderRef.current = null;
    };
  }, []);
  // Dev-only console driver (window.__processingDemo) that seeds a synthetic
  // meeting note parked in a transcription-processing stage so the
  // ProcessingProgressIndicator can be inspected without a real recording.
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    let cancelled = false;
    let dispose: (() => void) | undefined;
    void import("../lib/processing-progress-demo").then(({ registerProcessingProgressDemo }) => {
      if (cancelled) return;
      ({ dispose } = registerProcessingProgressDemo({
        seedNote: (note) => {
          dispatch({ type: "noteLoaded", note });
          setActiveView("meetings");
        },
      }));
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, []);
  // Dev-only console driver (window.__recordNoticesDemo) that parks the
  // recorder-area notices (consent reminder, source warning, mic-blocked) on the
  // selected note without a real recording, so their styling can be inspected.
  // The synthetic status runs under RECORD_NOTICES_DEMO_SESSION_ID, which the
  // status poll and the pause/resume/finish handlers skip so no backend call
  // fires; consent pinning bypasses the recorder bar's reveal/auto-hide timers.
  const [recordNoticesConsentPinned, setRecordNoticesConsentPinned] = useState(false);
  const [recordNoticesMicOverride, setRecordNoticesMicOverride] = useState<boolean | null>(null);
  const recordNoticesDemoRef = useRef<RecordNoticesDemoApi | null>(null);
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    let cancelled = false;
    void import("../lib/record-notices-demo").then(({ registerRecordNoticesDemo }) => {
      if (cancelled) return;
      recordNoticesDemoRef.current = registerRecordNoticesDemo({
        seedNote: (note) => {
          dispatch({ type: "noteLoaded", note });
          setActiveView("meetings");
        },
        setStatus: (status) => {
          // Defense in depth: never let the demo's synthetic status stomp a real
          // recording, even if the driver's hasRealRecording check somehow raced.
          const active = recordingStatusRef.current;
          if (active && active.sessionId !== RECORD_NOTICES_DEMO_SESSION_ID) return;
          if (status) {
            dispatch({ type: "recordingStatusChanged", status });
            setRecordingNote(status.noteId);
          } else {
            dispatch({ type: "recordingStatusCleared" });
            setRecordingNote(undefined);
            setLiveTranscriptEvents([]);
          }
        },
        setConsentPinned: setRecordNoticesConsentPinned,
        setMicOverride: setRecordNoticesMicOverride,
        getSelectedNoteId,
        hasRealRecording: () => {
          const active = recordingStatusRef.current;
          return !!active && active.sessionId !== RECORD_NOTICES_DEMO_SESSION_ID;
        },
      });
    });
    return () => {
      cancelled = true;
      recordNoticesDemoRef.current?.dispose();
      recordNoticesDemoRef.current = null;
    };
  }, [setRecordingNote]);
  // Dev console driver (window.__toastDemo) that fires each toast variant so
  // the toast styling can be inspected without walking a real flow.
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    let cancelled = false;
    let dispose: (() => void) | undefined;
    void import("../lib/toast-demo").then(({ registerToastDemo }) => {
      if (cancelled) return;
      ({ dispose } = registerToastDemo());
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, []);
  // Dev console driver (window.__clovySounds) for hearing the full recording
  // and agent sound family without walking each production lifecycle.
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    let cancelled = false;
    let dispose: (() => void) | undefined;
    void import("../lib/clovy-sounds-demo").then(({ registerClovySoundsDemo }) => {
      if (cancelled) return;
      ({ dispose } = registerClovySoundsDemo());
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, []);
  // Sessions with a finishRecording call in flight; guards stop double-clicks.
  const finishingSessionsRef = useRef<Set<string>>(new Set());

  return {
    demoRecorderStatus,
    recordNoticesConsentPinned,
    recordNoticesMicOverride,
    recordNoticesDemoRef,
    finishingSessionsRef,
  };
}
