import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { t } from "../i18n";
import { messageFromError } from "../lib/errors";
import {
  checkClovyUpdate,
  postponeUpdateInstall,
  relaunchClovy,
  updatesEnabled,
  type ClovyUpdate,
} from "../lib/updater";
import type { UpdateHubProps } from "./app-effects/update-ui";
import {
  INITIAL_UPDATE_STATUS_DISPLAY,
  UP_TO_DATE_STATUS,
  checkForClovyUpdate,
  nextUpdateStep,
  prepareClovyUpdate,
  updateCheckShowsStatus,
  updateStatusDisplayReducer,
  type UpdateCheckMode,
  type UpdateInstallProgress,
  type UpdatePromptPayload,
} from "./update-decision";
import type { UseAppUpdateActionsDependencies } from "./use-app-update-actions-types";

// "Clovy is up to date." is a confirmation, not a call to action: linger, then
// hide on its own. Failures persist until dismissed.
const UP_TO_DATE_DISMISS_MS = 4000;
// Soft-exit window: the update-popover-out animation runs var(--t-med) (160ms);
// the status clears just after it finishes.
const UP_TO_DATE_EXIT_MS = 220;
// Dictation lives in a ref, so a waiting update re-reads it on this cadence.
const CAPTURE_POLL_MS = 1000;

// Owns every update state: the launch check, the manual check from Settings,
// the offer, the owner's answer, the install and the restart. Only a release
// build checks; in a development build `updatesEnabled` stays false and
// nothing here runs.
export function useAppUpdateActions({
  appBlocked,
  recordingActive,
  dictationWorkflowActiveRef,
  flushNotes,
}: UseAppUpdateActionsDependencies) {
  const [enabled, setEnabled] = useState(false);
  const [offer, setOffer] = useState<UpdatePromptPayload<ClovyUpdate> | null>(null);
  const [accepted, setAccepted] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [ready, setReady] = useState(false);
  const [relaunching, setRelaunching] = useState(false);
  // Render-only flag for a visible manual check; checkingRef guards both modes.
  const [checking, setChecking] = useState(false);
  const [progress, setProgress] = useState<UpdateInstallProgress | null>(null);
  const [dictationActive, setDictationActive] = useState(false);
  // The package is downloaded and Rust holds the install until a capture
  // started during the download ends.
  const [installWaiting, setInstallWaiting] = useState(false);
  // "Later" while Rust waits: the install call then rejects, quietly.
  const installPostponedRef = useRef(false);
  const [display, dispatchDisplay] = useReducer(
    updateStatusDisplayReducer,
    INITIAL_UPDATE_STATUS_DISPLAY,
  );
  const setStatus = useCallback((status: string | null, failed = false) => {
    dispatchDisplay({ type: "show", status, failed });
  }, []);
  const checkingRef = useRef(false);
  // Read by runUpdateCheck so it keeps a stable identity.
  const busyRef = useRef(false);
  busyRef.current = preparing || ready || relaunching;

  useEffect(() => {
    let active = true;
    void updatesEnabled()
      .then((value) => {
        if (active) setEnabled(value);
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, []);

  const runUpdateCheck = useCallback(
    (mode: UpdateCheckMode) => {
      if (!enabled || checkingRef.current || busyRef.current) return;
      checkingRef.current = true;
      const showsStatus = updateCheckShowsStatus(mode);
      if (showsStatus) {
        setChecking(true);
        setStatus(t("app.update.checking"));
      }
      void checkForClovyUpdate(
        {
          check: checkClovyUpdate,
          offer: (payload) => {
            setOffer(payload);
            setAccepted(false);
            setStatus(null);
          },
          reportNoUpdate: () => setStatus(UP_TO_DATE_STATUS),
          reportFailure: (message) => setStatus(t("app.update.checkFailed", { message }), true),
        },
        mode,
      ).finally(() => {
        checkingRef.current = false;
        if (showsStatus) setChecking(false);
      });
    },
    [enabled, setStatus],
  );

  // Launch check: silent unless it finds an update, once per session.
  const launchCheckedRef = useRef(false);
  useEffect(() => {
    if (!enabled || appBlocked || launchCheckedRef.current) return;
    launchCheckedRef.current = true;
    runUpdateCheck("launch");
  }, [appBlocked, enabled, runUpdateCheck]);

  // Auto-dismiss only the up-to-date confirmation: linger, play the soft exit,
  // then clear. Any status change cancels the pending hide.
  useEffect(() => {
    if (display.status !== UP_TO_DATE_STATUS) return;
    const leaveTimer = window.setTimeout(() => {
      dispatchDisplay({ type: "beginUpToDateExit" });
    }, UP_TO_DATE_DISMISS_MS);
    const clearTimer = window.setTimeout(() => {
      dispatchDisplay({ type: "clearUpToDate" });
    }, UP_TO_DATE_DISMISS_MS + UP_TO_DATE_EXIT_MS);
    return () => {
      window.clearTimeout(leaveTimer);
      window.clearTimeout(clearTimer);
    };
  }, [display.status]);

  // Dictation state lives in a ref. Every step reads it synchronously before
  // acting; this poll only notices when a take the update waits for ends.
  useEffect(() => {
    if (!accepted) return;
    setDictationActive(dictationWorkflowActiveRef.current);
    const timer = window.setInterval(() => {
      setDictationActive(dictationWorkflowActiveRef.current);
    }, CAPTURE_POLL_MS);
    return () => window.clearInterval(timer);
  }, [accepted, dictationWorkflowActiveRef]);

  const step = nextUpdateStep({
    offered: offer !== null,
    accepted,
    preparing,
    ready,
    relaunching,
    captureActive: recordingActive || dictationActive,
  });

  useEffect(() => {
    if ((step === "install" || step === "relaunch") && dictationWorkflowActiveRef.current) {
      setDictationActive(true);
      return;
    }
    if (step === "install" && offer) {
      setPreparing(true);
      setProgress(null);
      setStatus(t("app.update.downloading"));
      installPostponedRef.current = false;
      void prepareClovyUpdate({
        update: offer.update,
        reportProgress: (next) => {
          // Installing (Rust took the slot) ends the wait and its "later".
          setInstallWaiting(next.state === "waitingForCapture");
          if (next.state === "waitingForCapture") {
            setProgress(null);
            setStatus(null);
            return;
          }
          setProgress(next);
          setStatus(
            next.state === "installing" ? t("app.update.preparing") : t("app.update.downloading"),
          );
        },
        reportReady: () => {
          setPreparing(false);
          setInstallWaiting(false);
          setReady(true);
          setProgress(null);
          setStatus(null);
        },
        reportFailure: (message) => {
          // The staged update is consumed: a retry starts with a new check.
          setPreparing(false);
          setInstallWaiting(false);
          setAccepted(false);
          setOffer(null);
          setProgress(null);
          if (installPostponedRef.current) {
            installPostponedRef.current = false;
            setStatus(null);
            return;
          }
          setStatus(t("app.update.failed", { message }), true);
        },
      });
      return;
    }
    if (step === "relaunch") {
      setRelaunching(true);
      setStatus(null);
      void flushNotes()
        .then(relaunchClovy)
        .catch((error) => {
          setRelaunching(false);
          setAccepted(false);
          setStatus(t("app.update.relaunchFailed", { message: messageFromError(error) }), true);
        });
    }
  }, [dictationWorkflowActiveRef, flushNotes, offer, setStatus, step]);

  const hub: UpdateHubProps = {
    offer,
    accepted,
    ready,
    waitingForCapture: step === "wait" || installWaiting,
    status: display.status,
    failed: display.failed,
    statusLeaving: display.leaving,
    checking,
    preparing,
    relaunching,
    progress,
    onInstall: () => {
      setDictationActive(dictationWorkflowActiveRef.current);
      setAccepted(true);
    },
    onPostpone: () => {
      if (installWaiting) {
        // Rust drops the downloaded package and rejects the install call. If
        // the install already took its slot, Rust refuses the postpone and
        // the install's own outcome stays visible.
        installPostponedRef.current = true;
        void postponeUpdateInstall().catch(() => {
          installPostponedRef.current = false;
        });
        return;
      }
      setAccepted(false);
      // Nothing downloaded yet: hide the offer until the next check. An
      // installed update stays as a restart card.
      if (!ready) setOffer(null);
    },
    onRelaunch: () => {
      setDictationActive(dictationWorkflowActiveRef.current);
      setAccepted(true);
    },
    onDismissStatus: () => setStatus(null),
  };

  return {
    updatesEnabled: enabled,
    runUpdateCheck,
    checkingForUpdates: checking,
    updateHub: hub,
  };
}
