import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { reconcileLiveTranscriptEvents } from "../lib/live-transcript-preview";
import { messageFromError } from "../lib/errors";
import {
  getNote,
  listNotes,
  NOTE_PROCESSING_PROGRESS_EVENT,
  type DiscardedTranscriptSpanDto,
  type TranscriptDto,
  type NoteListItemDto,
  type NoteProcessingProgressDto,
  type ProcessingStatus,
} from "../lib/tauri";
import { PROCESSING_DEMO_NOTE_ID } from "./processing-demo-ids";
import type { UseNoteProcessingEventsDependencies } from "./use-note-processing-events-types";
import { getCurrentDataPartitionName } from "../lib/data-partition";

export const NOTE_PROCESSING_RECONCILE_INTERVAL_MS = 12_000;

function progressIdentity(progress: NoteProcessingProgressDto) {
  return `${progress.recordingSessionId}\0${progress.stage}\0${progress.revision}`;
}

function isActiveProcessingStatus(status: ProcessingStatus) {
  return status === "transcribing" || status === "generating";
}

function activeProcessingNotes(notes: NoteListItemDto[]) {
  return notes.filter(
    (note) =>
      isActiveProcessingStatus(note.processingStatus) &&
      !(import.meta.env.DEV && note.id === PROCESSING_DEMO_NOTE_ID),
  );
}

export function useNoteProcessingEvents(dependencies: UseNoteProcessingEventsDependencies) {
  const {
    dispatch,
    noteSaveController,
    folderChangesRef,
    notes,
    selectedNote,
    liveTranscriptEvents,
    setError,
    setLiveTranscriptEvents,
  } = dependencies;
  const discardedByNoteRef = useRef(new Map<string, DiscardedTranscriptSpanDto[]>());
  const [discardRevision, setDiscardRevision] = useState(0);
  const selectedNoteRef = useRef(selectedNote);
  selectedNoteRef.current = selectedNote;
  const visibleNotesRef = useRef(notes);
  visibleNotesRef.current = notes;
  const activeNotesRef = useRef(new Map<string, NoteListItemDto>());
  const reconcileInFlightRef = useRef(new Map<string, Promise<void>>());
  const pendingTerminalHydrationsRef = useRef(new Map<string, NoteProcessingProgressDto>());
  const terminalHydrationInFlightRef = useRef(new Map<string, Promise<void>>());
  const terminalHydrationRetryRef = useRef(new Map<string, number>());
  const deletedNoteIdsRef = useRef(new Set<string>());
  const refreshCompletedNoteRef = useRef<(progress: NoteProcessingProgressDto) => Promise<void>>(
    async () => undefined,
  );
  const disposedRef = useRef(false);
  const activeNotes = activeProcessingNotes(notes);
  const activeProcessingKey = activeNotes
    .map((note) => note.id)
    .sort()
    .join("\0");

  useEffect(() => {
    let disposed = false;
    let latestRequest = 0;
    const noteRequests = new Map<string, number>();
    let unlisten: (() => void) | undefined;
    void listen<{ profile: string; kind: string; id?: string; deleted?: boolean }>(
      "clovy-data-changed",
      ({ payload }) => {
        if (
          !["notes", "projects"].includes(payload.kind) ||
          payload.profile !== getCurrentDataPartitionName()
        )
          return;
        const noteId =
          (payload.kind === "notes" ? payload.id : undefined) ?? selectedNoteRef.current?.id;
        const deleted = payload.kind === "notes" && !!payload.id && !!payload.deleted;
        const request = ++latestRequest;
        if (noteId) noteRequests.set(noteId, request);
        const isCurrent = () => !disposed && payload.profile === getCurrentDataPartitionName();
        if (noteId && deleted) {
          deletedNoteIdsRef.current.add(noteId);
          pendingTerminalHydrationsRef.current.delete(noteId);
          activeNotesRef.current.delete(noteId);
          const retry = terminalHydrationRetryRef.current.get(noteId);
          if (retry !== undefined) window.clearTimeout(retry);
          terminalHydrationRetryRef.current.delete(noteId);
          noteSaveController.discard(noteId);
          dispatch({ type: "noteDeleted", noteId });
        }
        void (async () => {
          try {
            if (noteId && !deleted && !deletedNoteIdsRef.current.has(noteId)) {
              await noteSaveController.flush(noteId);
              if (!isCurrent() || noteRequests.get(noteId) !== request) return;
              const editVersion = noteSaveController.getEditVersion(noteId);
              const folderVersion = folderChangesRef.current.notes.get(noteId);
              const deletedProjects = new Map(folderChangesRef.current.deletedProjects);
              const note = await getNote(noteId);
              if (
                isCurrent() &&
                ((payload.kind === "notes" && !!payload.id) ||
                  selectedNoteRef.current?.id === noteId) &&
                noteRequests.get(noteId) === request &&
                !deletedNoteIdsRef.current.has(noteId)
              ) {
                const folderChanged = folderVersion !== folderChangesRef.current.notes.get(noteId);
                const folderIds = note.folderIds.filter(
                  (id) =>
                    folderChangesRef.current.deletedProjects.get(id) === deletedProjects.get(id),
                );
                if (
                  editVersion === noteSaveController.getEditVersion(noteId) &&
                  !noteSaveController.hasPending(noteId)
                ) {
                  dispatch({
                    type: folderChanged ? "noteUpdatedPreservingFolders" : "noteUpdated",
                    note: { ...note, folderIds },
                  });
                } else if (!folderChanged) {
                  dispatch({ type: "noteFoldersReconciled", noteId, folderIds });
                }
              }
            }
            const editVersions = new Map(
              visibleNotesRef.current.map((note) => [
                note.id,
                noteSaveController.getEditVersion(note.id),
              ]),
            );
            const folderVersions = new Map(folderChangesRef.current.notes);
            const deletedProjects = new Map(folderChangesRef.current.deletedProjects);
            const pendingAtRead = new Set(
              visibleNotesRef.current
                .filter((note) => noteSaveController.hasPending(note.id))
                .map((note) => note.id),
            );
            const response = await listNotes();
            if (isCurrent() && request === latestRequest) {
              dispatch({
                type: "notesInvalidated",
                notes: response.items.map((note) => ({
                  ...note,
                  folderIds: note.folderIds.filter(
                    (id) =>
                      folderChangesRef.current.deletedProjects.get(id) === deletedProjects.get(id),
                  ),
                })),
                preserveIds: response.items
                  .filter(
                    (note) =>
                      pendingAtRead.has(note.id) ||
                      noteSaveController.hasPending(note.id) ||
                      (editVersions.get(note.id) ?? 0) !==
                        noteSaveController.getEditVersion(note.id),
                  )
                  .map((note) => note.id),
                preserveFolderIdsForNoteIds: response.items
                  .filter(
                    (note) =>
                      folderVersions.get(note.id) !== folderChangesRef.current.notes.get(note.id),
                  )
                  .map((note) => note.id),
                deletedId: deleted ? noteId : undefined,
              });
            }
          } catch (error) {
            if (
              isCurrent() &&
              (!noteId ||
                (!deletedNoteIdsRef.current.has(noteId) && noteRequests.get(noteId) === request))
            )
              setError(messageFromError(error));
          }
        })();
      },
    )
      .then((cleanup) => {
        if (disposed) cleanup();
        else unlisten = cleanup;
      })
      .catch(() => {
        if (!disposed) console.warn("Clovy note refresh subscription is unavailable.");
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [dispatch, noteSaveController, setError]);

  useEffect(() => {
    activeNotesRef.current = new Map(
      activeProcessingNotes(notes)
        .filter((note) => !deletedNoteIdsRef.current.has(note.id))
        .map((note) => [note.id, note]),
    );
  }, [notes]);

  useEffect(() => {
    disposedRef.current = false;
    return () => {
      disposedRef.current = true;
      for (const timeout of terminalHydrationRetryRef.current.values()) {
        window.clearTimeout(timeout);
      }
      terminalHydrationRetryRef.current.clear();
    };
  }, []);

  const reconcilePreview = useCallback(
    (noteId: string, persisted: TranscriptDto[]) => {
      const discarded = discardedByNoteRef.current.get(noteId) ?? [];
      setLiveTranscriptEvents((current) => {
        const relevant = current.filter((event) => event.noteId === noteId);
        const retained = reconcileLiveTranscriptEvents(relevant, persisted, discarded);
        if (retained === relevant) return current;
        const kept = new Set(retained);
        return current.filter((event) => event.noteId !== noteId || kept.has(event));
      });
    },
    [setLiveTranscriptEvents],
  );

  useEffect(() => {
    // A terminal event can add spans without changing the preview array.
    void discardRevision;
    for (const [noteId, spans] of discardedByNoteRef.current) {
      const retained = spans.filter((span) =>
        liveTranscriptEvents.some(
          (event) =>
            event.noteId === noteId &&
            event.sessionId === span.sessionId &&
            event.source === span.source &&
            event.startMs < span.endMs &&
            span.startMs < event.endMs,
        ),
      );
      if (retained.length) discardedByNoteRef.current.set(noteId, retained);
      else discardedByNoteRef.current.delete(noteId);
    }
  }, [liveTranscriptEvents, discardRevision]);

  useEffect(() => {
    if (selectedNote) reconcilePreview(selectedNote.id, selectedNote.sourceTranscripts ?? []);
  }, [selectedNote, reconcilePreview]);

  const clearTerminalHydration = useCallback((progress: NoteProcessingProgressDto) => {
    const pending = pendingTerminalHydrationsRef.current.get(progress.noteId);
    if (!pending || progressIdentity(pending) !== progressIdentity(progress)) return;
    pendingTerminalHydrationsRef.current.delete(progress.noteId);
    const timeout = terminalHydrationRetryRef.current.get(progress.noteId);
    if (timeout !== undefined) {
      window.clearTimeout(timeout);
      terminalHydrationRetryRef.current.delete(progress.noteId);
    }
  }, []);

  const scheduleTerminalHydrationRetry = useCallback((progress: NoteProcessingProgressDto) => {
    if (terminalHydrationRetryRef.current.has(progress.noteId)) return;
    const timeout = window.setTimeout(() => {
      terminalHydrationRetryRef.current.delete(progress.noteId);
      const pending = pendingTerminalHydrationsRef.current.get(progress.noteId);
      if (
        disposedRef.current ||
        !pending ||
        progressIdentity(pending) !== progressIdentity(progress)
      ) {
        return;
      }
      void refreshCompletedNoteRef.current(pending);
    }, NOTE_PROCESSING_RECONCILE_INTERVAL_MS);
    terminalHydrationRetryRef.current.set(progress.noteId, timeout);
  }, []);

  const refreshCompletedNote = useCallback(
    (progress: NoteProcessingProgressDto) => {
      if (deletedNoteIdsRef.current.has(progress.noteId)) return Promise.resolve();
      const pending = terminalHydrationInFlightRef.current.get(progress.noteId);
      if (pending) return pending;

      let shouldRetry = false;
      const hydration = (async () => {
        const profile = getCurrentDataPartitionName();
        try {
          // PR #917 made editor persistence asynchronous. Drain that note's
          // optimistic edits before hydrating the generated result so the done
          // snapshot cannot replace a newer local title or body.
          await noteSaveController.flush(progress.noteId);
          if (
            deletedNoteIdsRef.current.has(progress.noteId) ||
            profile !== getCurrentDataPartitionName()
          )
            return;
          const editVersion = noteSaveController.getEditVersion(progress.noteId);
          const note = await getNote(progress.noteId);
          const latest = pendingTerminalHydrationsRef.current.get(progress.noteId);
          if (
            disposedRef.current ||
            deletedNoteIdsRef.current.has(progress.noteId) ||
            profile !== getCurrentDataPartitionName() ||
            !latest ||
            progressIdentity(latest) !== progressIdentity(progress)
          ) {
            return;
          }
          if (
            editVersion !== noteSaveController.getEditVersion(progress.noteId) ||
            noteSaveController.hasPending(progress.noteId)
          ) {
            shouldRetry = true;
            return;
          }
          if (note.updatedAt < progress.revision) {
            shouldRetry = true;
            return;
          }
          reconcilePreview(note.id, note.sourceTranscripts ?? []);
          dispatch({ type: "noteUpdated", note });
          clearTerminalHydration(progress);
        } catch (error) {
          if (
            deletedNoteIdsRef.current.has(progress.noteId) ||
            profile !== getCurrentDataPartitionName()
          )
            return;
          shouldRetry = true;
          if (!disposedRef.current) setError(messageFromError(error));
        }
      })();

      terminalHydrationInFlightRef.current.set(progress.noteId, hydration);
      void hydration.finally(() => {
        if (terminalHydrationInFlightRef.current.get(progress.noteId) === hydration) {
          terminalHydrationInFlightRef.current.delete(progress.noteId);
        }
        const latest = pendingTerminalHydrationsRef.current.get(progress.noteId);
        if (
          disposedRef.current ||
          !latest ||
          progressIdentity(latest) !== progressIdentity(progress)
        ) {
          if (latest && !disposedRef.current) void refreshCompletedNoteRef.current(latest);
          return;
        }
        if (shouldRetry) scheduleTerminalHydrationRetry(progress);
      });
      return hydration;
    },
    [
      clearTerminalHydration,
      dispatch,
      noteSaveController,
      scheduleTerminalHydrationRetry,
      setError,
      reconcilePreview,
    ],
  );
  refreshCompletedNoteRef.current = refreshCompletedNote;

  const reconcileNote = useCallback(
    (noteId: string) => {
      if (deletedNoteIdsRef.current.has(noteId)) return Promise.resolve();
      const pending = reconcileInFlightRef.current.get(noteId);
      if (pending) return pending;

      const reconciliation = (async () => {
        const profile = getCurrentDataPartitionName();
        try {
          // The safety path can race an editor save just like the done event.
          // Flush first so recovery never replaces a newer optimistic edit.
          await noteSaveController.flush(noteId);
          if (deletedNoteIdsRef.current.has(noteId) || profile !== getCurrentDataPartitionName())
            return;
          const editVersion = noteSaveController.getEditVersion(noteId);
          const note = await getNote(noteId);
          const activeSnapshot = activeNotesRef.current.get(noteId);
          if (
            disposedRef.current ||
            !activeSnapshot ||
            deletedNoteIdsRef.current.has(noteId) ||
            profile !== getCurrentDataPartitionName() ||
            editVersion !== noteSaveController.getEditVersion(noteId) ||
            noteSaveController.hasPending(noteId)
          )
            return;
          // The backstop detects a missed terminal event; it deliberately does
          // not restore progressive full-note hydration while processing.
          if (isActiveProcessingStatus(note.processingStatus)) return;
          if (note.updatedAt < activeSnapshot.updatedAt) return;
          reconcilePreview(note.id, note.sourceTranscripts ?? []);
          dispatch({ type: "noteUpdated", note });
        } catch (error) {
          if (
            !disposedRef.current &&
            !deletedNoteIdsRef.current.has(noteId) &&
            profile === getCurrentDataPartitionName()
          )
            setError(messageFromError(error));
        }
      })();

      reconcileInFlightRef.current.set(noteId, reconciliation);
      void reconciliation.finally(() => {
        if (reconcileInFlightRef.current.get(noteId) === reconciliation) {
          reconcileInFlightRef.current.delete(noteId);
        }
      });
      return reconciliation;
    },
    [dispatch, noteSaveController, setError, reconcilePreview],
  );

  const reconcileActiveNotes = useCallback(() => {
    for (const noteId of activeNotesRef.current.keys()) {
      void reconcileNote(noteId);
    }
  }, [reconcileNote]);

  const reconcilePendingTerminalHydrations = useCallback(() => {
    for (const progress of pendingTerminalHydrationsRef.current.values()) {
      void refreshCompletedNote(progress);
    }
  }, [refreshCompletedNote]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;

    void listen<NoteProcessingProgressDto>(NOTE_PROCESSING_PROGRESS_EVENT, (event) => {
      const progress = event.payload;
      if (deletedNoteIdsRef.current.has(progress.noteId)) return;
      dispatch({
        type: "noteProcessingStageChanged",
        noteId: progress.noteId,
        processingStatus: progress.processingStatus,
      });
      if (progress.stage === "done") {
        const discarded = (progress.discardedSpans ?? []).filter(
          (span) => span.sessionId === progress.recordingSessionId,
        );
        if (discarded.length) {
          const previous = discardedByNoteRef.current.get(progress.noteId) ?? [];
          const unique = new Map(
            [...previous, ...discarded].map((span) => [
              JSON.stringify([span.sessionId, span.source, span.startMs, span.endMs]),
              span,
            ]),
          );
          discardedByNoteRef.current.set(progress.noteId, [...unique.values()]);
          setDiscardRevision((revision) => revision + 1);
          const note = selectedNoteRef.current;
          reconcilePreview(
            progress.noteId,
            note?.id === progress.noteId ? (note.sourceTranscripts ?? []) : [],
          );
        }
        pendingTerminalHydrationsRef.current.set(progress.noteId, progress);
        void refreshCompletedNote(progress);
      } else {
        const pending = pendingTerminalHydrationsRef.current.get(progress.noteId);
        if (pending) clearTerminalHydration(pending);
      }
    })
      .then((cleanup) => {
        if (disposed) cleanup();
        else unlisten = cleanup;
      })
      .catch(() => {
        if (!disposed)
          console.warn(
            "Clovy note processing subscription is unavailable; reconciliation remains active.",
          );
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [clearTerminalHydration, dispatch, refreshCompletedNote, reconcilePreview]);

  useEffect(() => {
    const reconcileProcessing = () => {
      reconcileActiveNotes();
      reconcilePendingTerminalHydrations();
    };
    const onFocus = () => reconcileProcessing();
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") reconcileProcessing();
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [reconcileActiveNotes, reconcilePendingTerminalHydrations]);

  useEffect(() => {
    if (!activeProcessingKey) return;
    const interval = window.setInterval(
      reconcileActiveNotes,
      NOTE_PROCESSING_RECONCILE_INTERVAL_MS,
    );
    return () => window.clearInterval(interval);
  }, [activeProcessingKey, reconcileActiveNotes]);
}
