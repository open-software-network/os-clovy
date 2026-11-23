import { assignNoteToFolder, deleteFolder, removeNoteFromFolder } from "../lib/tauri";
import { messageFromError } from "../lib/errors";
import type { CreateAppDomainActionsDependencies } from "./app-domain-actions-types";
import { markFolderChange } from "./folder-changes";

export function createAppDomainActions(dependencies: CreateAppDomainActionsDependencies) {
  const { dispatch, noteSaveController, folderChangesRef, setError, state } = dependencies;

  async function handleDeleteFolder(folderId: string) {
    try {
      // Deleting a project strips its association from any notes and agent
      // sessions but never deletes them — they stay in your library.
      await deleteFolder(folderId, false);
      markFolderChange(folderChangesRef.current.deletedProjects, folderId);
      dispatch({ type: "folderDeleted", folderId });
    } catch (err) {
      setError(messageFromError(err));
      throw err;
    }
  }

  async function handleRemoveNoteFromFolder(
    noteId: string,
    folderId: string,
    options?: { rethrow?: boolean },
  ) {
    try {
      await noteSaveController.flush(noteId);
      const note = await removeNoteFromFolder(noteId, folderId);
      markFolderChange(folderChangesRef.current.notes, noteId);
      dispatch({ type: "noteUpdated", note });
    } catch (err) {
      setError(messageFromError(err));
      if (options?.rethrow) throw err;
    }
  }

  // Single-folder semantics: a note belongs to at most one folder. Strip any
  // existing folder assignments before adding the target. Legacy notes with
  // multiple folders get normalized on the next move.
  async function handleSetNoteFolder(
    noteId: string,
    folderId: string,
    options?: { rethrow?: boolean },
  ) {
    const note = state.notes.find((n) => n.id === noteId);
    if (!note) return;
    if (note.folderIds.length === 1 && note.folderIds[0] === folderId) return;
    try {
      await noteSaveController.flush(noteId);
      for (const existing of note.folderIds) {
        if (existing === folderId) continue;
        const updated = await removeNoteFromFolder(noteId, existing);
        markFolderChange(folderChangesRef.current.notes, noteId);
        dispatch({ type: "noteUpdated", note: updated });
      }
      if (!note.folderIds.includes(folderId)) {
        const updated = await assignNoteToFolder(noteId, folderId);
        markFolderChange(folderChangesRef.current.notes, noteId);
        dispatch({ type: "noteUpdated", note: updated });
      }
    } catch (err) {
      setError(messageFromError(err));
      if (options?.rethrow) throw err;
    }
  }

  return {
    handleDeleteFolder,
    handleRemoveNoteFromFolder,
    handleSetNoteFolder,
  };
}
