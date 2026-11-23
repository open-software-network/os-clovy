import type { NoteSaveController } from "./note-save-controller";
import type { NotesAction, NotesState } from "./state/app-state";
import type * as React from "react";

export type CreateAppDomainActionsDependencies = {
  dispatch: React.Dispatch<NotesAction>;
  noteSaveController: NoteSaveController;
  folderChangesRef: import("./folder-changes").FolderChanges;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  state: NotesState;
};
