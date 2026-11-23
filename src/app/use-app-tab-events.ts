import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef } from "react";
import { NOTE_CALENDAR_CONTEXT_UPDATED_EVENT } from "../lib/tauri";
import { isPrimaryShortcut } from "../lib/platform";
import { getCurrentDataPartitionName } from "../lib/data-partition";
import { CLOSE_TAB_EVENT } from "../lib/menu-bar";
import type { NoteDto } from "../lib/tauri";
import type { UseAppTabEventsDependencies } from "./use-app-tab-events-types";

export function useAppTabEvents(dependencies: UseAppTabEventsDependencies) {
  const {
    activateTab,
    activeTabId,
    activeTabIdRef,
    calendarContextNotePartitionsRef,
    calendarContextNoteUpdatesRef,
    closeTab,
    cycleTab,
    dispatch,
    openNewTab,
    pendingCalendarContextAdoptionsRef,
    setTabs,
    tabs,
  } = dependencies;

  function pruneDeletedNoteTabs(removedIds: Set<string>) {
    setTabs((prev) =>
      prev.filter(
        (tab) =>
          tab.id === activeTabId ||
          !(tab.nav.view === "meetings" && tab.nav.noteId && removedIds.has(tab.nav.noteId)),
      ),
    );
  }

  useEffect(() => {
    let aborted = false;
    let unlisten: (() => void) | undefined;
    void listen(CLOSE_TAB_EVENT, () => {
      if (document.querySelector('[role="dialog"]')) return;
      closeTab(activeTabIdRef.current);
    }).then((cleanup) => {
      if (aborted) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      aborted = true;
      unlisten?.();
    };
  }, [closeTab]);

  useEffect(() => {
    let aborted = false;
    let unlisten: (() => void) | undefined;
    void listen<NoteDto>(NOTE_CALENDAR_CONTEXT_UPDATED_EVENT, (event) => {
      const notePartition = calendarContextNotePartitionsRef.current.get(event.payload.id);
      calendarContextNotePartitionsRef.current.delete(event.payload.id);
      if (notePartition !== getCurrentDataPartitionName()) return;
      if (pendingCalendarContextAdoptionsRef.current.delete(event.payload.id)) {
        calendarContextNoteUpdatesRef.current.set(event.payload.id, event.payload);
      }
      dispatch({ type: "noteUpdated", note: event.payload });
    }).then((cleanup) => {
      if (aborted) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      aborted = true;
      unlisten?.();
    };
  }, []);

  // Tab keyboard shortcuts: ⌘T new, ⌘W close, ⌘[ / ⌘] cycle, ⌘1-9 jump
  // (9 = last).
  // isPrimaryShortcut handles the cross-platform modifier (⌘ on mac, Ctrl on
  // Windows) and rejects Alt/Shift. No dependency array — re-bound each render
  // so it closes over current tabs, matching the search/new-note effects below.
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (!isPrimaryShortcut(event)) return;
      if (document.querySelector('[role="dialog"]')) return;
      const key = event.key;
      if (key.toLowerCase() === "t") {
        event.preventDefault();
        openNewTab();
        return;
      }
      if (key.toLowerCase() === "w") {
        event.preventDefault();
        closeTab(activeTabId);
        return;
      }
      if (key === "]" || key === "}") {
        event.preventDefault();
        cycleTab(1);
        return;
      }
      if (key === "[" || key === "{") {
        event.preventDefault();
        cycleTab(-1);
        return;
      }
      if (/^[1-9]$/.test(key)) {
        event.preventDefault();
        const n = Number(key);
        const target = n >= tabs.length ? tabs[tabs.length - 1] : tabs[n - 1];
        if (target) activateTab(target.id);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  // Modifier state for the click that's about to fire a navigation. A
  // capture-phase listener records it before React's bubble-phase handlers run,
  // so any nav surface (sidebar, notes list, command prompt) can open in a new
  // tab via ⌘/Ctrl-click or middle-click without threading flags through props.
  const newTabIntentRef = useRef(false);
  useEffect(() => {
    const record = (event: MouseEvent) => {
      newTabIntentRef.current = event.metaKey || event.ctrlKey || event.button === 1;
    };
    window.addEventListener("click", record, true);
    window.addEventListener("auxclick", record, true);
    return () => {
      window.removeEventListener("click", record, true);
      window.removeEventListener("auxclick", record, true);
    };
  }, []);
  // Reads and clears the intent: true when the triggering click wanted a new tab.
  const takeNewTabIntent = useCallback(() => {
    const intent = newTabIntentRef.current;
    newTabIntentRef.current = false;
    return intent;
  }, []);

  return {
    pruneDeletedNoteTabs,
    takeNewTabIntent,
  };
}
