import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { nextDictationWorkflowActive, parseDictationHelperEvent } from "../lib/dictation-events";
import { stringPayloadValue } from "./app-helpers";
import type { UseDictationEventsDependencies } from "./use-dictation-events-types";

export function useDictationEvents(dependencies: UseDictationEventsDependencies) {
  const { dictationWorkflowActiveRef, setAccessibilityStatus, setActiveView, setMicrophoneStatus } =
    dependencies;

  useEffect(() => {
    let aborted = false;
    let unlisten: (() => void) | undefined;
    void listen<string>("dictation-event", (event) => {
      const helperEvent = parseDictationHelperEvent(event.payload);
      if (!helperEvent) return;
      dictationWorkflowActiveRef.current = nextDictationWorkflowActive(
        dictationWorkflowActiveRef.current,
        helperEvent.type,
        helperEvent.payload?.preserveActiveTake === true,
      );
      if (helperEvent.type === "agent_session_prompt") {
        setActiveView("today");
        return;
      }
      if (
        helperEvent.type !== "permission_status" &&
        helperEvent.type !== "dictation_diagnostics"
      ) {
        return;
      }
      const microphone = stringPayloadValue(helperEvent.payload?.microphone);
      const accessibility = stringPayloadValue(helperEvent.payload?.accessibility);
      if (microphone) setMicrophoneStatus(microphone);
      if (accessibility) setAccessibilityStatus(accessibility);
    }).then((cleanup) => {
      if (aborted) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      aborted = true;
      unlisten?.();
    };
  }, [dictationWorkflowActiveRef, setAccessibilityStatus, setActiveView, setMicrophoneStatus]);
}
