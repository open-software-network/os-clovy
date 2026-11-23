import type {
  BootstrapResponse,
  FolderDto,
  NoteDto,
  NotePatchDto,
  NoteListItemDto,
  ProcessingStatus,
  RecoverableRecordingDto,
  RecordingStatusDto,
} from "../../lib/tauri";

export type NotesState = {
  folders: FolderDto[];
  notes: NoteListItemDto[];
  /** Confirmed deletions fence every asynchronous producer of note snapshots. */
  deletedNoteIds?: ReadonlySet<string>;
  selectedNoteId?: string;
  selectedNote?: NoteDto;
  selectedFolderId?: string;
  recordingStatus?: RecordingStatusDto;
  activeRecoveries: RecoverableRecordingDto[];
};

export type NotesAction =
  | { type: "bootstrapLoaded"; payload: BootstrapResponse }
  | { type: "noteLoaded"; note: NoteDto }
  | { type: "noteUpdated"; note: NoteDto }
  | { type: "noteUpdatedPreservingFolders"; note: NoteDto }
  | { type: "noteFoldersReconciled"; noteId: string; folderIds: string[] }
  | {
      type: "notePatched";
      noteId: string;
      patch: Partial<Omit<NotePatchDto, "id">>;
    }
  | {
      type: "noteProcessingStageChanged";
      noteId: string;
      processingStatus: ProcessingStatus;
    }
  | { type: "noteProcessingUpdated"; note: NoteDto }
  | { type: "recordingStatusChanged"; status: RecordingStatusDto }
  | { type: "recordingStatusCleared" }
  | { type: "recordingSessionLost"; sessionId: string }
  | { type: "folderCreated"; folder: FolderDto }
  | { type: "folderRenamed"; folder: FolderDto }
  | { type: "folderUpdated"; folder: FolderDto }
  | { type: "folderDeleted"; folderId: string }
  | { type: "folderSelected"; folderId?: string }
  | { type: "foldersLoaded"; folders: FolderDto[] }
  | { type: "notesLoaded"; notes: NoteListItemDto[] }
  | {
      type: "notesInvalidated";
      notes: NoteListItemDto[];
      deletedId?: string;
      preserveIds?: string[];
      preserveFolderIdsForNoteIds?: string[];
    }
  | { type: "noteDeleted"; noteId: string }
  | { type: "recoveriesUpdated"; recoveries: RecoverableRecordingDto[] }
  | { type: "recoveryRemoved"; sessionId: string };

export function createInitialState(): NotesState {
  return {
    folders: [],
    notes: [],
    deletedNoteIds: new Set(),
    activeRecoveries: [],
  };
}

export function notesReducer(state: NotesState, action: NotesAction): NotesState {
  if ("note" in action && state.deletedNoteIds?.has(action.note.id)) return state;
  if (
    "noteId" in action &&
    action.type !== "noteDeleted" &&
    state.deletedNoteIds?.has(action.noteId)
  )
    return state;
  switch (action.type) {
    case "bootstrapLoaded": {
      const notes = action.payload.notes.filter((note) => !state.deletedNoteIds?.has(note.id));
      const selectedNoteId = state.selectedNoteId ?? notes[0]?.id;
      return {
        ...state,
        folders: sortFolders(action.payload.folders),
        notes,
        activeRecoveries: action.payload.activeRecoveries,
        selectedNoteId,
      };
    }
    case "noteLoaded":
      return {
        ...upsertNote(state, action.note),
        selectedNoteId: action.note.id,
        selectedNote: action.note,
      };
    // Incidental updates (autosave, folder moves, polls): the snapshot may
    // carry stale DB state, so the processing status is merged to never
    // regress an in-flight pipeline or reopen a terminal one.
    case "noteUpdated":
      return applyNoteUpdate(state, mergeNoteUpdate(state, action.note));
    case "noteUpdatedPreservingFolders": {
      const current =
        state.selectedNote?.id === action.note.id
          ? state.selectedNote
          : state.notes.find((note) => note.id === action.note.id);
      const note = { ...action.note, folderIds: current?.folderIds ?? action.note.folderIds };
      return applyNoteUpdate(state, mergeNoteUpdate(state, note));
    }
    case "noteFoldersReconciled":
      return {
        ...state,
        notes: state.notes.map((note) =>
          note.id === action.noteId ? { ...note, folderIds: action.folderIds } : note,
        ),
        selectedNote:
          state.selectedNote?.id === action.noteId
            ? { ...state.selectedNote, folderIds: action.folderIds }
            : state.selectedNote,
      };
    case "notePatched":
      return applyNotePatch(state, action.noteId, action.patch);
    case "noteProcessingStageChanged":
      return applyNoteProcessingStage(state, action.noteId, action.processingStatus);
    // Authoritative results of commands whose purpose is to change the
    // processing status (retry, recover, finish recording). Applied as-is so
    // they can restart processing on a note that already sits in a terminal
    // status — failed → transcribing on retry, ready → transcribing when
    // stacking another take — which the merge would otherwise swallow,
    // leaving the note stuck stale with no shimmer and no polling.
    case "noteProcessingUpdated":
      return applyNoteUpdate(state, action.note);
    case "recordingStatusChanged":
      return {
        ...state,
        recordingStatus: action.status,
      };
    case "recordingStatusCleared":
      return {
        ...state,
        recordingStatus: undefined,
      };
    // The backend no longer knows this session (e.g. it was finalized or the
    // capture process restarted). Only clear if the UI still shows that exact
    // session so a fresh recording started in the meantime is untouched.
    case "recordingSessionLost":
      return state.recordingStatus?.sessionId === action.sessionId
        ? { ...state, recordingStatus: undefined }
        : state;
    case "folderCreated":
      return {
        ...state,
        folders: sortFolders([
          ...state.folders.filter((folder) => folder.id !== action.folder.id),
          action.folder,
        ]),
      };
    case "folderRenamed":
    case "folderUpdated":
      return {
        ...state,
        folders: sortFolders(
          state.folders.map((folder) => (folder.id === action.folder.id ? action.folder : folder)),
        ),
      };
    case "folderDeleted":
      return {
        ...state,
        folders: state.folders.filter((folder) => folder.id !== action.folderId),
        selectedFolderId:
          state.selectedFolderId === action.folderId ? undefined : state.selectedFolderId,
        notes: state.notes.map((note) =>
          note.folderIds.includes(action.folderId)
            ? {
                ...note,
                folderIds: note.folderIds.filter((id) => id !== action.folderId),
              }
            : note,
        ),
        selectedNote: state.selectedNote?.folderIds.includes(action.folderId)
          ? {
              ...state.selectedNote,
              folderIds: state.selectedNote.folderIds.filter((id) => id !== action.folderId),
            }
          : state.selectedNote,
      };
    case "foldersLoaded":
      return {
        ...state,
        folders: sortFolders(action.folders),
      };
    case "folderSelected":
      return {
        ...state,
        selectedFolderId: action.folderId,
        selectedNoteId: undefined,
        selectedNote: undefined,
      };
    case "notesLoaded": {
      const notes = action.notes.filter((note) => !state.deletedNoteIds?.has(note.id));
      return {
        ...state,
        notes,
        selectedNoteId: notes[0]?.id,
        selectedNote: undefined,
      };
    }
    case "noteDeleted":
      return {
        ...state,
        deletedNoteIds: new Set([...(state.deletedNoteIds ?? []), action.noteId]),
        notes: state.notes.filter((note) => note.id !== action.noteId),
        selectedNoteId: state.selectedNoteId === action.noteId ? undefined : state.selectedNoteId,
        selectedNote: state.selectedNoteId === action.noteId ? undefined : state.selectedNote,
      };
    case "notesInvalidated":
      return {
        ...state,
        notes: action.notes
          .filter((note) => !state.deletedNoteIds?.has(note.id))
          .map((note) => {
            if (
              !action.preserveIds?.includes(note.id) &&
              !action.preserveFolderIdsForNoteIds?.includes(note.id)
            )
              return note;
            const current = state.notes.find((item) => item.id === note.id);
            if (action.preserveIds?.includes(note.id)) return current ?? note;
            if (action.preserveFolderIdsForNoteIds?.includes(note.id) && current) {
              return { ...note, folderIds: current.folderIds };
            }
            return note;
          }),
        selectedNoteId:
          state.selectedNoteId === action.deletedId ? undefined : state.selectedNoteId,
        selectedNote: state.selectedNoteId === action.deletedId ? undefined : state.selectedNote,
      };
    case "recoveriesUpdated":
      return {
        ...state,
        activeRecoveries: action.recoveries,
      };
    case "recoveryRemoved":
      return {
        ...state,
        activeRecoveries: state.activeRecoveries.filter(
          (recovery) => recovery.sessionId !== action.sessionId,
        ),
      };
    default:
      return state;
  }
}

function applyNotePatch(
  state: NotesState,
  noteId: string,
  patch: Partial<Omit<NotePatchDto, "id">>,
): NotesState {
  const selectedNote =
    state.selectedNote?.id === noteId
      ? mergeDefined<NoteDto>(state.selectedNote, patch)
      : state.selectedNote;
  const notes = state.notes.map((note) => {
    if (note.id !== noteId) return note;
    return mergeDefined(note, {
      title: patch.title,
      preview: patch.preview,
      updatedAt: patch.updatedAt,
    });
  });
  return {
    ...state,
    notes,
    selectedNote,
  };
}

function applyNoteProcessingStage(
  state: NotesState,
  noteId: string,
  processingStatus: ProcessingStatus,
): NotesState {
  return {
    ...state,
    selectedNote:
      state.selectedNote?.id === noteId
        ? { ...state.selectedNote, processingStatus }
        : state.selectedNote,
    notes: state.notes.map((note) => (note.id === noteId ? { ...note, processingStatus } : note)),
  };
}

function mergeDefined<T extends object>(current: T, patch: Partial<T>): T {
  const defined = Object.fromEntries(
    Object.entries(patch).filter(([, value]) => value !== undefined && value !== null),
  ) as Partial<T>;
  return { ...current, ...defined };
}

function applyNoteUpdate(state: NotesState, note: NoteDto): NotesState {
  return {
    ...upsertNote(state, note),
    selectedNote: state.selectedNoteId === note.id ? note : state.selectedNote,
  };
}

function mergeNoteUpdate(state: NotesState, note: NoteDto): NoteDto {
  const current =
    state.selectedNote?.id === note.id
      ? state.selectedNote
      : state.notes.find((item) => item.id === note.id);
  if (!current) return note;

  const processingStatus = mergeProcessingStatus(current.processingStatus, note.processingStatus);
  if (processingStatus === note.processingStatus) return note;
  if (
    isTerminalProcessingStatus(current.processingStatus) &&
    !isTerminalProcessingStatus(note.processingStatus)
  ) {
    return current;
  }

  return {
    ...note,
    processingStatus,
  };
}

function mergeProcessingStatus(
  current: ProcessingStatus,
  incoming: ProcessingStatus,
): ProcessingStatus {
  if (isTerminalProcessingStatus(incoming)) {
    return incoming;
  }
  if (isTerminalProcessingStatus(current)) return current;

  const currentRank = activeProcessingRank(current);
  const incomingRank = activeProcessingRank(incoming);
  if (
    currentRank >= activeProcessingRank("transcribing") &&
    incomingRank >= 0 &&
    incomingRank < currentRank
  ) {
    return current;
  }

  return incoming;
}

function isTerminalProcessingStatus(status: ProcessingStatus): boolean {
  return status === "ready" || status === "failed" || status === "recoverable";
}

function activeProcessingRank(status: ProcessingStatus): number {
  switch (status) {
    case "draft":
      return 0;
    case "recording":
      return 1;
    case "validating":
      return 2;
    case "transcribing":
      return 3;
    case "generating":
      return 4;
    default:
      return -1;
  }
}

function upsertNote(state: NotesState, note: NoteDto): NotesState {
  const existing = state.notes.filter((item) => item.id !== note.id);
  return {
    ...state,
    notes: [toListItem(note), ...existing].sort((a, b) => b.createdAt.localeCompare(a.createdAt)),
  };
}

function sortFolders(folders: FolderDto[]): FolderDto[] {
  return [...folders].sort((a, b) =>
    a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
  );
}

function toListItem(note: NoteDto): NoteListItemDto {
  return {
    id: note.id,
    title: note.title,
    preview: note.preview,
    processingStatus: note.processingStatus,
    folderIds: note.folderIds,
    createdAt: note.createdAt,
    updatedAt: note.updatedAt,
    durationMs: note.durationMs,
  };
}
