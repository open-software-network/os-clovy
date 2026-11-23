import { useSyncExternalStore } from "react";
import { t } from "../i18n";
import { messageFromError } from "./errors";
import {
  type BrowserSession,
  browserHumanTakeover,
  browserResumeControl,
  browserSessions,
  browserStopSession,
} from "./tauri";

type BrowserSessionsState = {
  sessions: BrowserSession[];
  actionError?: string;
  stateError?: string;
  busy?: string;
};

const INITIAL_STATE: BrowserSessionsState = { sessions: [] };
let state = INITIAL_STATE;
const subscribers = new Set<() => void>();
let timer: number | undefined;
let revision = 0;
let generation = 0;
let operation: number | undefined;
let stopping = false;

function publish(update: Partial<BrowserSessionsState>) {
  state = { ...state, ...update };
  for (const subscriber of subscribers) subscriber();
}

async function refresh() {
  const current = ++revision;
  try {
    const next = await browserSessions();
    if (!Array.isArray(next)) throw new Error(t("settingsPanels.browserUse.stateUnavailable"));
    if (current === revision) publish({ sessions: next, stateError: undefined });
  } catch (cause) {
    if (current === revision) publish({ stateError: messageFromError(cause) });
  }
}

function onFocus() {
  void refresh();
}

function subscribe(subscriber: () => void) {
  subscribers.add(subscriber);
  if (subscribers.size === 1) {
    void refresh();
    timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void refresh();
    }, 2000);
    window.addEventListener("focus", onFocus);
  }
  return () => {
    subscribers.delete(subscriber);
    if (subscribers.size === 0) {
      if (timer !== undefined) window.clearInterval(timer);
      timer = undefined;
      window.removeEventListener("focus", onFocus);
      // Outstanding operations and their receipts belong to the app, not a view.
    }
  };
}

function getSnapshot() {
  return state;
}

function getServerSnapshot() {
  return INITIAL_STATE;
}

/** Acknowledges a displayed error only; never repeats or authorizes an action. */
export function dismissBrowserSessionError() {
  publish({ actionError: undefined });
}

async function stop(sessionId: string) {
  // Stop bypasses takeover/resume from every surface sharing this store.
  if (stopping) return;
  stopping = true;
  const current = ++generation;
  operation = undefined;
  revision++;
  publish({ busy: sessionId, actionError: undefined });
  try {
    await browserStopSession(sessionId);
    if (generation !== current) return;
    await refresh();
  } catch (cause) {
    if (generation !== current) return;
    publish({ actionError: messageFromError(cause) });
    await refresh();
  } finally {
    stopping = false;
    if (generation === current) publish({ busy: undefined });
  }
}

async function act(sessionId: string, action: "takeover" | "resume" | "stop") {
  if (action === "stop") return stop(sessionId);
  if (operation !== undefined || stopping) return;
  const current = ++generation;
  operation = current;
  publish({ busy: sessionId, actionError: undefined });
  try {
    if (action === "takeover") await browserHumanTakeover(sessionId);
    else await browserResumeControl(sessionId);
    if (generation !== current) return;
    await refresh();
  } catch (cause) {
    if (generation !== current) return;
    publish({ actionError: messageFromError(cause) });
    // Delivery may be uncertain. Re-read state; never replay an operation.
    await refresh();
  } finally {
    if (operation === current) operation = undefined;
    if (generation === current) publish({ busy: undefined });
  }
}

/** One app-owned snapshot and operation lifetime across Plugins and the shell. */
export function useBrowserSessions() {
  const snapshot = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
  return {
    sessions: snapshot.sessions,
    error: snapshot.actionError || snapshot.stateError,
    actionError: snapshot.actionError,
    busy: snapshot.busy,
    act,
    dismissActionError: dismissBrowserSessionError,
  };
}
