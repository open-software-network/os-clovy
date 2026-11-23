import { useCallback, useEffect, useMemo, useRef } from "react";
import { defaultNav, makeTabId, navEquals, normalizeTabNav, type TabNav } from "./tabs/tabs";
import { getNote } from "../lib/tauri";
import { messageFromError } from "../lib/errors";
import type { UseAppNavigationDependencies } from "./use-app-navigation-types";

export function useAppNavigation(dependencies: UseAppNavigationDependencies) {
  const {
    activeTabId,
    activeTabIdRef,
    activeView,
    activeViewRef,
    dispatch,
    originAllNotes,
    originFolderId,
    restoreTargetRef,
    selectedNoteId,
    setActiveTabId,
    setActiveView,
    setError,
    setFolderReturnTarget,
    setOriginAllNotes,
    setOriginFolderId,
    setSettingsReturnView,
    setTabs,
    state,
    tabs,
    tabsRef,
  } = dependencies;

  const liveNav = useMemo<TabNav>(
    () => ({
      view: activeView,
      noteId: activeView === "meetings" ? selectedNoteId : undefined,
      originFolderId: activeView === "meetings" ? originFolderId : undefined,
      originAllNotes: activeView === "meetings" ? originAllNotes : undefined,
      folderId: activeView === "folders" ? state.selectedFolderId : undefined,
    }),
    [activeView, selectedNoteId, originFolderId, originAllNotes, state.selectedFolderId],
  );

  // Mirror live navigation into the active tab. While a restore is in flight we
  // hold off until live nav settles onto the target, then release — this keeps
  // an async note load mid-switch from stamping a half-built snapshot onto the
  // tab we're moving to.
  useEffect(() => {
    if (restoreTargetRef.current) {
      if (navEquals(restoreTargetRef.current, liveNav)) {
        restoreTargetRef.current = null;
      }
      return;
    }
    setTabs((prev) =>
      prev.map((tab) => {
        if (tab.id !== activeTabId) return tab;
        return navEquals(tab.nav, liveNav) ? tab : { ...tab, nav: liveNav };
      }),
    );
  }, [liveNav, activeTabId]);

  useEffect(() => {
    tabsRef.current = tabs;
  }, [tabs]);

  useEffect(() => {
    activeTabIdRef.current = activeTabId;
  }, [activeTabId]);

  // Keep the latest selected note id reachable from applyNav without making it
  // a dependency (which would rebuild the callback on every note change).
  const selectedNoteIdRef = useRef(selectedNoteId);
  useEffect(() => {
    selectedNoteIdRef.current = selectedNoteId;
  }, [selectedNoteId]);

  // Push a snapshot into live state. Used by tab switch / open only; in-tab
  // navigation keeps flowing through the existing handlers untouched.
  const applyNav = useCallback((snapshot: Parameters<typeof normalizeTabNav>[0]) => {
    const nav = normalizeTabNav(snapshot);
    // The guard only needs to bridge the async note fetch — everything else
    // applies synchronously. So hold capture only while a fetch is in flight
    // and release it when the fetch settles (success or failure), never tying
    // release to a target that might be unreachable (a deleted session/note).
    restoreTargetRef.current = nav;
    setOriginFolderId(nav.view === "meetings" ? nav.originFolderId : undefined);
    setOriginAllNotes(nav.view === "meetings" ? !!nav.originAllNotes : false);
    // The "back to <note>" breadcrumb target isn't part of a tab's snapshot,
    // so clear it on every restore — otherwise it leaks from the tab that set
    // it into whatever tab we switch to.
    setFolderReturnTarget(undefined);
    if (nav.view === "folders") {
      dispatch({ type: "folderSelected", folderId: nav.folderId });
    }
    // Mirror openSettings: a settings tab (e.g. cmd-clicked open) needs a
    // return view recorded so exiting Settings lands where it came from.
    if (nav.view === "settings") {
      const returnView = activeViewRef.current;
      if (returnView !== "settings") setSettingsReturnView(returnView);
    }
    const needsNoteLoad =
      nav.view === "meetings" && !!nav.noteId && selectedNoteIdRef.current !== nav.noteId;
    if (needsNoteLoad) {
      const noteId = nav.noteId!;
      void getNote(noteId)
        .then((note) => dispatch({ type: "noteLoaded", note }))
        .catch((err: unknown) => setError(messageFromError(err)))
        .finally(() => {
          // A newer restore may have superseded this one — only release the
          // guard if it's still ours.
          if (restoreTargetRef.current === nav) {
            restoreTargetRef.current = null;
          }
        });
    } else {
      // Nothing async to wait for — let capture resume immediately.
      restoreTargetRef.current = null;
    }
    setActiveView(nav.view);
  }, []);
  const applyNavRef = useRef(applyNav);
  useEffect(() => {
    applyNavRef.current = applyNav;
  }, [applyNav]);

  function activateTab(id: string) {
    if (id === activeTabId) return;
    const target = tabs.find((tab) => tab.id === id);
    if (!target) return;
    setActiveTabId(id);
    applyNav(target.nav);
  }

  // Open a fresh tab on the given snapshot and focus it. The active tab's own
  // snapshot was already captured by the mirror effect, so nothing is lost.
  function openTab(snapshot: Parameters<typeof normalizeTabNav>[0]) {
    const nav = normalizeTabNav(snapshot);
    const id = makeTabId();
    const index = tabs.findIndex((tab) => tab.id === activeTabId);
    const next = [...tabs];
    // Drop the new tab in just to the right of the active one, like a browser.
    next.splice(index < 0 ? tabs.length : index + 1, 0, { id, nav });
    setTabs(next);
    setActiveTabId(id);
    applyNav(nav);
  }

  const openNewTab = () => openTab(defaultNav());

  const closeTab = useCallback((id: string) => {
    const currentTabs = tabsRef.current;
    const currentActiveTabId = activeTabIdRef.current;

    if (currentTabs.length <= 1) {
      // Never leave the strip empty — reset the sole tab to Today.
      const fresh = { id: makeTabId(), nav: defaultNav() };
      tabsRef.current = [fresh];
      activeTabIdRef.current = fresh.id;
      setTabs([fresh]);
      setActiveTabId(fresh.id);
      applyNavRef.current(fresh.nav);
      return;
    }
    const index = currentTabs.findIndex((tab) => tab.id === id);
    if (index < 0) return;
    const next = currentTabs.filter((tab) => tab.id !== id);
    tabsRef.current = next;
    setTabs(next);
    if (id === currentActiveTabId) {
      // Focus the right neighbor, falling back to the left — browser behavior.
      const neighbor = next[index] ?? next[index - 1];
      if (neighbor) {
        activeTabIdRef.current = neighbor.id;
        setActiveTabId(neighbor.id);
        applyNavRef.current(neighbor.nav);
      }
    }
  }, []);

  return {
    applyNav,
    activateTab,
    openTab,
    openNewTab,
    closeTab,
    selectedNoteIdRef,
  };
}
