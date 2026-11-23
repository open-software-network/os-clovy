import { useEffect } from "react";
import { invalidateNoteTabs } from "./tabs/tabs";
import { getNote, listFolders, listNotes } from "../lib/tauri";
import { messageFromError } from "../lib/errors";
import { useClovyDataChanged } from "../lib/clovy-data-events";
import type { UseDataPartitionRefreshDependencies } from "./use-data-partition-refresh-types";

export function useDataPartitionRefresh(dependencies: UseDataPartitionRefreshDependencies) {
  const {
    currentDataPartitionName,
    activeViewRef,
    appBlocked,
    bootstrapped,
    crossPartitionRecordingNoteIdRef,
    dispatch,
    lastDataPartitionRef,
    lastDataPartitionRefreshRevisionRef,
    dataPartitionRefreshRevision,
    recordingNoteIdRef,
    setActiveView,
    setError,
    setFolderReturnTarget,
    setOriginAllNotes,
    setOriginFolderId,
    setTabs,
    tabsRef,
  } = dependencies;

  useClovyDataChanged("notes", (change) => {
    if (!change.deleted || !change.id) return;
    const next = invalidateNoteTabs(tabsRef.current, new Set([change.id]));
    if (next !== tabsRef.current) {
      tabsRef.current = next;
      setTabs(next);
    }
  });

  useClovyDataChanged("projects", async (_change, isCurrent) => {
    try {
      const folders = await listFolders();
      if (!isCurrent()) return;
      // Notes and conversations have their own refresh owners. A project
      // snapshot must not replace their newer deletion/update results.
      dispatch({ type: "foldersLoaded", folders });
    } catch (error) {
      if (isCurrent()) setError(messageFromError(error));
    }
  });

  useEffect(() => {
    if (appBlocked || !bootstrapped) return;
    const previous = lastDataPartitionRef.current;
    const refreshRequested =
      lastDataPartitionRefreshRevisionRef.current !== dataPartitionRefreshRevision;
    lastDataPartitionRef.current = currentDataPartitionName;
    lastDataPartitionRefreshRevisionRef.current = dataPartitionRefreshRevision;
    if (!refreshRequested && (previous === undefined || previous === currentDataPartitionName)) {
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const [notesResponse, folders] = await Promise.all([listNotes(), listFolders()]);
        if (cancelled) return;
        const visibleNoteIds = new Set(notesResponse.items.map((note) => note.id));
        const recordingNoteId = recordingNoteIdRef.current;
        crossPartitionRecordingNoteIdRef.current =
          recordingNoteId && !visibleNoteIds.has(recordingNoteId) ? recordingNoteId : undefined;
        const invalidNoteIds = new Set<string>();
        for (const tab of tabsRef.current) {
          const noteId = tab.nav.view === "meetings" ? tab.nav.noteId : undefined;
          if (noteId && noteId !== recordingNoteId && !visibleNoteIds.has(noteId)) {
            invalidNoteIds.add(noteId);
          }
        }
        const nextTabs = invalidateNoteTabs(tabsRef.current, invalidNoteIds);
        if (nextTabs !== tabsRef.current) {
          tabsRef.current = nextTabs;
          setTabs(nextTabs);
        }
        // The previous partition's folder selection and origins point at rows
        // the new partition cannot see. Clear them before the new lists land.
        dispatch({ type: "folderSelected", folderId: undefined });
        dispatch({ type: "foldersLoaded", folders });
        dispatch({ type: "notesLoaded", notes: notesResponse.items });
        setOriginFolderId(undefined);
        setOriginAllNotes(false);
        setFolderReturnTarget(undefined);
        const nextNoteId = recordingNoteIdRef.current ?? notesResponse.items[0]?.id;
        if (nextNoteId) {
          const note = await getNote(nextNoteId);
          if (!cancelled) dispatch({ type: "noteLoaded", note });
        } else if (!cancelled) {
          const currentView = activeViewRef.current;
          if (currentView === "meetings" || currentView === "all-notes") {
            setActiveView("notes");
          }
        }
      } catch (err) {
        if (!cancelled) setError(messageFromError(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [currentDataPartitionName, appBlocked, bootstrapped, dataPartitionRefreshRevision]);
}
