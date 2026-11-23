import type * as React from "react";
import type { LiveTranscriptEventDto, NoteDto, NoteListItemDto } from "../lib/tauri";
import type { NoteSaveController } from "./note-save-controller";
import type { NotesAction } from "./state/app-state";

export type UseNoteProcessingEventsDependencies = {
  dispatch: React.Dispatch<NotesAction>;
  noteSaveController: NoteSaveController;
  folderChangesRef: import("./folder-changes").FolderChanges;
  notes: NoteListItemDto[];
  selectedNote: NoteDto | undefined;
  liveTranscriptEvents: LiveTranscriptEventDto[];
  setLiveTranscriptEvents: React.Dispatch<React.SetStateAction<LiveTranscriptEventDto[]>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
};
