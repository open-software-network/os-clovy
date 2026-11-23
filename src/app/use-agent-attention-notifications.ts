import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect } from "react";
import {
  AGENT_RUN_SETTLED_EVENT,
  AGENT_SESSION_STATUS_EVENT,
  type AgentRunSettledDetail,
  type AgentSessionStatusDetail,
} from "../lib/agent-events";
import {
  notifyAgentRunSettled,
  notifyAgentSessionStatus,
  type AgentAttentionContext,
} from "../lib/agent-notifications";
import { agentRuntimeBindings } from "../lib/tauri";
import { getAgentSoundsEnabled } from "../lib/agent-sound-settings";
import type { UseAgentAttentionNotificationsDependencies } from "./use-agent-attention-notifications-types";

export function useAgentAttentionNotifications(
  dependencies: UseAgentAttentionNotificationsDependencies,
) {
  const { activeViewRef, dictationWorkflowActiveRef, recordingStatusRef } = dependencies;

  useEffect(() => {
    let disposed = false;
    async function attentionContextFor(): Promise<AgentAttentionContext> {
      let windowFocused = document.hasFocus();
      try {
        const appWindow = getCurrentWindow();
        if (typeof appWindow.isFocused === "function") {
          windowFocused = await appWindow.isFocused();
        }
      } catch {
        // Browser previews do not expose a Tauri window; document focus is enough.
      }
      const away = document.visibilityState !== "visible" || !windowFocused;
      const recordingState = recordingStatusRef.current?.state;
      const recordingCaptureActive =
        recordingState === "recording" ||
        recordingState === "paused" ||
        recordingState === "finalizing" ||
        recordingState === "validating";
      return {
        away,
        agentHudEnabled: false,
        viewingSession: !away && activeViewRef.current === "routines",
        captureActive: recordingCaptureActive || dictationWorkflowActiveRef.current,
        soundsEnabled: getAgentSoundsEnabled(),
        routineResult: true,
      };
    }

    async function notifyRoutine<Detail extends { sessionId?: string; title?: string }>(
      detail: Detail,
      notify: (detail: Detail, context: AgentAttentionContext) => Promise<boolean>,
    ) {
      if (!detail.sessionId) return;
      const run = await agentRuntimeBindings.getSession(detail.sessionId).catch(() => null);
      if (disposed || (run?.source !== "routine" && run?.source !== "legacy_routine")) return;
      const context = await attentionContextFor();
      if (!disposed) await notify({ ...detail, title: run.title }, context);
    }

    const handleAgentStatus = (event: Event) => {
      const detail = (event as CustomEvent<AgentSessionStatusDetail>).detail;
      if (!detail || (detail.status !== "waitingForUser" && detail.status !== "failed")) return;
      void notifyRoutine(detail, notifyAgentSessionStatus);
    };
    const handleAgentRunSettled = (event: Event) => {
      const detail = (event as CustomEvent<AgentRunSettledDetail>).detail;
      if (!detail) return;
      void notifyRoutine(detail, notifyAgentRunSettled);
    };
    window.addEventListener(AGENT_SESSION_STATUS_EVENT, handleAgentStatus);
    window.addEventListener(AGENT_RUN_SETTLED_EVENT, handleAgentRunSettled);
    return () => {
      disposed = true;
      window.removeEventListener(AGENT_SESSION_STATUS_EVENT, handleAgentStatus);
      window.removeEventListener(AGENT_RUN_SETTLED_EVENT, handleAgentRunSettled);
    };
  }, [activeViewRef, dictationWorkflowActiveRef, recordingStatusRef]);
}
