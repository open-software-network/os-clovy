import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { ROUTINE_OPEN_EVENT, requestRoutineRunOpen } from "../lib/agent-routine-history";
import { agentOpenReady, routineOpenReady } from "../lib/tauri";
import { AGENT_NEW_SESSION_EVENT, AGENT_OPEN_EVENT } from "../lib/agent-events";
import { requestTodayOpen, TODAY_OPEN_EVENT, todayOpenReady } from "../lib/day-intelligence";
import {
  AGENT_MENU_BAR_NEW_SESSION_EVENT,
  AGENT_MENU_BAR_OPEN_SESSION_EVENT,
} from "../lib/menu-bar";
import type { SidebarView } from "../components/sidebar/Sidebar";

export function useAppExternalEvents(setActiveView: (view: SidebarView) => void) {
  useEffect(() => {
    let aborted = false;
    const unlisteners: Array<() => void> = [];
    const openToday = () => setActiveView("today");
    const acknowledgeLegacyOpen = () => {
      openToday();
      void agentOpenReady().catch(() => null);
    };

    async function install() {
      const registrations = await Promise.allSettled([
        listen(AGENT_OPEN_EVENT, acknowledgeLegacyOpen),
        listen(AGENT_MENU_BAR_NEW_SESSION_EVENT, openToday),
        listen(AGENT_MENU_BAR_OPEN_SESSION_EVENT, openToday),
      ]);
      for (const registration of registrations) {
        if (registration.status !== "fulfilled") continue;
        if (aborted) registration.value();
        else unlisteners.push(registration.value);
      }
      if (aborted) return;
      const queued = await agentOpenReady().catch(() => null);
      if (!aborted && queued) openToday();
    }

    window.addEventListener(AGENT_OPEN_EVENT, acknowledgeLegacyOpen);
    window.addEventListener(AGENT_NEW_SESSION_EVENT, openToday);
    void install();
    return () => {
      aborted = true;
      for (const unlisten of unlisteners) unlisten();
      window.removeEventListener(AGENT_OPEN_EVENT, acknowledgeLegacyOpen);
      window.removeEventListener(AGENT_NEW_SESSION_EVENT, openToday);
    };
  }, [setActiveView]);

  useEffect(() => {
    let aborted = false;
    let unlisten: (() => void) | undefined;

    function openRun(sessionId: string | null | undefined) {
      if (aborted || !sessionId) return;
      setActiveView("routines");
      requestRoutineRunOpen(sessionId);
    }

    void listen<{ sessionId?: string }>(ROUTINE_OPEN_EVENT, ({ payload }) => {
      openRun(payload.sessionId);
      void routineOpenReady().catch(() => null);
    })
      .then((cleanup) => {
        if (aborted) {
          cleanup();
          return;
        }
        unlisten = cleanup;
        void routineOpenReady()
          .then(openRun)
          .catch(() => {});
      })
      .catch(() => {});
    return () => {
      aborted = true;
      unlisten?.();
    };
  }, [setActiveView]);

  // Activity notifications (day summary ready, failures) open the "Today"
  // view at that day's summary. A click that launched the app is drained
  // from the backend queue once the listener exists.
  useEffect(() => {
    let aborted = false;
    let unlisten: (() => void) | undefined;
    const openToday = (day: string | null | undefined) => {
      if (aborted || !day) return;
      setActiveView("today");
      requestTodayOpen(day);
    };
    void listen<{ day?: string }>(TODAY_OPEN_EVENT, (event) => {
      openToday(event.payload?.day);
      // The backend keeps the click queued until drained (webview reloads).
      void todayOpenReady().catch(() => null);
    }).then((cleanup) => {
      if (aborted) cleanup();
      else unlisten = cleanup;
      void todayOpenReady()
        .then(openToday)
        .catch(() => {});
    });
    return () => {
      aborted = true;
      unlisten?.();
    };
  }, [setActiveView]);
}
