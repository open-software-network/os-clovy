import { t } from "../i18n";
import { deleteNote, deleteNotes, getNote, listNotes } from "../lib/tauri";
import { messageFromError } from "../lib/errors";
import type { NoteEditablePatch } from "../lib/tauri";
import type { CreateNoteActionsDependencies } from "./note-actions-types";

export function createNoteActions(dependencies: CreateNoteActionsDependencies) {
  const {
    dispatch,
    handleEmptyNotesAfterDelete,
    noteSaveController,
    pruneDeletedNoteTabs,
    setActiveView,
    setError,
    setFolderReturnTarget,
    setOriginAllNotes,
    setOriginFolderId,
    state,
  } = dependencies;

  async function handleDeleteNote(noteId: string) {
    if (state.recordingStatus) {
      setError(t("app.recording.stopBeforeDeletingNote"));
      return;
    }
    try {
      await noteSaveController.flush(noteId);
      noteSaveController.discard(noteId);
      await deleteNote(noteId);
      pruneDeletedNoteTabs(new Set([noteId]));
      const response = await listNotes();
      dispatch({ type: "notesLoaded", notes: response.items });
      const nextNoteId = response.items[0]?.id;
      if (nextNoteId) {
        const note = await getNote(nextNoteId);
        dispatch({ type: "noteLoaded", note });
      } else {
        handleEmptyNotesAfterDelete();
      }
    } catch (err) {
      setError(messageFromError(err));
    }
  }

  async function handleDeleteNotes(noteIds: string[]) {
    if (state.recordingStatus) {
      setError(t("app.recording.stopBeforeDeletingMeetings"));
      return;
    }
    try {
      await Promise.all(noteIds.map((noteId) => noteSaveController.flush(noteId)));
      for (const noteId of noteIds) {
        noteSaveController.discard(noteId);
      }
      await deleteNotes(noteIds);
      pruneDeletedNoteTabs(new Set(noteIds));
      const response = await listNotes();
      dispatch({ type: "notesLoaded", notes: response.items });
      const nextNoteId = response.items[0]?.id;
      if (nextNoteId) {
        const note = await getNote(nextNoteId);
        dispatch({ type: "noteLoaded", note });
      } else {
        handleEmptyNotesAfterDelete();
      }
    } catch (err) {
      setError(messageFromError(err));
    }
  }

  async function handleSelectNoteFromFolder(noteId: string, folderId: string) {
    try {
      const note = await getNote(noteId);
      dispatch({ type: "noteLoaded", note });
      setOriginFolderId(folderId);
      setOriginAllNotes(false);
      setFolderReturnTarget(undefined);
      setActiveView("meetings");
    } catch (err) {
      setError(messageFromError(err));
    }
  }

  function handleUpdateNote(noteId: string, patch: NoteEditablePatch) {
    dispatch({
      type: "notePatched",
      noteId,
      patch: { ...patch, updatedAt: new Date().toISOString() },
    });
    noteSaveController.queue(noteId, patch);
  }

  async function handleFlushNote(noteId: string) {
    try {
      await noteSaveController.flush(noteId);
    } catch (err) {
      setError(messageFromError(err));
    }
  }

  async function handleSaveNoteNow(noteId: string, patch: NoteEditablePatch) {
    dispatch({
      type: "notePatched",
      noteId,
      patch: { ...patch, updatedAt: new Date().toISOString() },
    });
    try {
      await noteSaveController.saveNow(noteId, patch);
    } catch (err) {
      setError(messageFromError(err));
    }
  }

  return {
    handleDeleteNote,
    handleDeleteNotes,
    handleFlushNote,
    handleSaveNoteNow,
    handleSelectNoteFromFolder,
    handleUpdateNote,
  };
}
