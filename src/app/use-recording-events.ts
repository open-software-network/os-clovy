import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { acknowledgeMeetingStartRequest, pendingMeetingStartRequest } from "../lib/tauri";
import { MEETING_START_TRANSCRIPTION_EVENT } from "../lib/events";
import { revealMainWindowForMeetingStartError } from "./app-helpers";
import {
  MEETING_START_LISTENER_RETRY_DELAYS_MS,
  meetingStartRequestExpiredMessage,
} from "./app-shell";
import type { UseRecordingEventsDependencies } from "./use-recording-events-types";

export function useRecordingEvents(dependencies: UseRecordingEventsDependencies) {
  const {
    drainPendingMeetingStartRef,
    meetingStartHandlerRef,
    meetingStartListenerRegisteredRef,
    meetingStartReadyRef,
    setError,
  } = dependencies;

  useEffect(() => {
    let aborted = false;
    let unlisten: (() => void) | undefined;
    let listenerRetryTimer: number | undefined;
    let drainRetryTimer: number | undefined;
    let drainRunning = false;
    let drainAgain = false;

    const scheduleDrainRetry = () => {
      if (aborted || drainRetryTimer !== undefined) return;
      drainRetryTimer = window.setTimeout(() => {
        drainRetryTimer = undefined;
        drainPendingRequest();
      }, 500);
    };

    function drainPendingRequest() {
      if (!meetingStartReadyRef.current || !meetingStartListenerRegisteredRef.current) return;
      if (drainRunning) {
        drainAgain = true;
        return;
      }
      drainRunning = true;
      void (async () => {
        let shouldRetry = false;
        try {
          const request = await pendingMeetingStartRequest();
          if (
            aborted ||
            !meetingStartReadyRef.current ||
            !meetingStartListenerRegisteredRef.current ||
            !request
          ) {
            return;
          }
          if (request.expired) {
            revealMainWindowForMeetingStartError();
            setError(meetingStartRequestExpiredMessage());
            const acknowledged = await acknowledgeMeetingStartRequest(request.requestId);
            shouldRetry = !acknowledged;
            return;
          }
          const terminal = await meetingStartHandlerRef.current(request.requestId, request.noteId);
          if (
            aborted ||
            !meetingStartReadyRef.current ||
            !meetingStartListenerRegisteredRef.current
          ) {
            return;
          }
          if (!terminal) {
            shouldRetry = true;
            return;
          }
          const acknowledged = await acknowledgeMeetingStartRequest(request.requestId);
          shouldRetry = !acknowledged;
        } catch {
          shouldRetry = true;
        } finally {
          drainRunning = false;
          if (!aborted) {
            if (drainAgain) {
              drainAgain = false;
              drainPendingRequest();
            } else if (shouldRetry) {
              scheduleDrainRetry();
            }
          }
        }
      })();
    }
    drainPendingMeetingStartRef.current = drainPendingRequest;

    const register = (attempt = 0) => {
      void listen(MEETING_START_TRANSCRIPTION_EVENT, drainPendingRequest)
        .then((cleanup) => {
          if (aborted) {
            cleanup();
            return;
          }
          unlisten = cleanup;
          meetingStartListenerRegisteredRef.current = true;
          drainPendingRequest();
        })
        .catch((error) => {
          if (!aborted) {
            console.warn("Failed to register the meeting start listener; retrying.", error);
            const retryDelay =
              MEETING_START_LISTENER_RETRY_DELAYS_MS[
                Math.min(attempt, MEETING_START_LISTENER_RETRY_DELAYS_MS.length - 1)
              ];
            listenerRetryTimer = window.setTimeout(() => register(attempt + 1), retryDelay);
          }
        });
    };

    register();
    return () => {
      aborted = true;
      meetingStartListenerRegisteredRef.current = false;
      if (listenerRetryTimer !== undefined) window.clearTimeout(listenerRetryTimer);
      if (drainRetryTimer !== undefined) window.clearTimeout(drainRetryTimer);
      unlisten?.();
    };
  }, []);
}
