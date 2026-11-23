import { act, renderHook, waitFor } from "@testing-library/react";
import { useReducer, useRef, useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NoteSaveController } from "../app/note-save-controller";
import { createInitialState, notesReducer } from "../app/state/app-state";
import { useDataPartitionRefresh } from "../app/use-data-partition-refresh";
import { useNoteProcessingEvents } from "../app/use-note-processing-events";
import type { Tab } from "../app/tabs/tabs";
import type { ClovyDataChange } from "../lib/clovy-data-events";
import { resetCurrentDataPartitionForTests } from "../lib/data-partition";
import type { FolderDto, LiveTranscriptEventDto, NoteDto } from "../lib/tauri";

const mocks = vi.hoisted(() => ({
  listen: vi.fn(),
  getNote: vi.fn(),
  listNotes: vi.fn(),
  listFolders: vi.fn(),
  listAgentSessions: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("../lib/tauri", async (original) => ({
  ...(await original<typeof import("../lib/tauri")>()),
  ...mocks,
}));
const listeners = new Set<(event: { payload: ClovyDataChange }) => void>();
const stamp = "2026-11-01T00:00:00Z";
const note: NoteDto = {
  id: "note",
  title: "Note",
  preview: "Note",
  activeTab: "notes",
  processingStatus: "ready",
  createdAt: stamp,
  updatedAt: stamp,
  folderIds: ["project"],
};
const folder: FolderDto = {
  id: "project",
  name: "Project",
  memoryDisabled: false,
  createdAt: stamp,
  updatedAt: stamp,
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((finish) => {
    resolve = finish;
  });
  return { promise, resolve };
}
async function emit(payload: ClovyDataChange) {
  await act(async () => {
    for (const listener of listeners) listener({ payload });
  });
}
function useSurfaces() {
  const [state, dispatch] = useReducer(notesReducer, {
    ...createInitialState(),
    notes: [note],
    selectedNote: note,
    selectedNoteId: note.id,
  });
  const [tabs, setTabs] = useState<Tab[]>([
    { id: "tab-note", nav: { view: "meetings", noteId: note.id } },
  ]);
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const activeViewRef = useRef<"meetings">("meetings");
  const [error, setError] = useState<string | null>(null);
  const [liveTranscriptEvents, setLiveTranscriptEvents] = useState<LiveTranscriptEventDto[]>([]);
  const controller = useRef(
    new NoteSaveController({
      persist: async () => {
        throw Error("Unexpected save");
      },
      debounceMs: 60_000,
    }),
  ).current;
  useNoteProcessingEvents({
    dispatch,
    noteSaveController: controller,
    folderChangesRef: useRef({ notes: new Map(), deletedProjects: new Map() }),
    notes: state.notes,
    selectedNote: state.selectedNote,
    liveTranscriptEvents,
    setLiveTranscriptEvents,
    setError,
  });
  useDataPartitionRefresh({
    currentDataPartitionName: "default",
    activeViewRef,
    appBlocked: false,
    bootstrapped: true,
    crossPartitionRecordingNoteIdRef: useRef(undefined),
    dispatch,
    lastDataPartitionRef: useRef("default"),
    lastDataPartitionRefreshRevisionRef: useRef(0),
    dataPartitionRefreshRevision: 0,
    recordingNoteIdRef: useRef(undefined),
    setActiveView: vi.fn(),
    setError,
    setFolderReturnTarget: vi.fn(),
    setOriginAllNotes: vi.fn(),
    setOriginFolderId: vi.fn(),
    setTabs,
    tabsRef,
  });
  return { state, tabs, error };
}
beforeEach(() => {
  vi.clearAllMocks();
  listeners.clear();
  resetCurrentDataPartitionForTests();
  mocks.listen.mockImplementation(async (name, callback) => {
    if (name !== "clovy-data-changed") return () => {};
    listeners.add(callback);
    return () => listeners.delete(callback);
  });
  mocks.listNotes.mockResolvedValue({ items: [], hasMore: false });
  mocks.getNote.mockResolvedValue(note);
  mocks.listFolders.mockResolvedValue([folder]);
});
describe("project refresh and confirmed deletion", () => {
  it("does not restore a deleted note or its tab from an older project refresh", async () => {
    const notes = deferred<{ items: NoteDto[]; hasMore: boolean }>();
    const folders = deferred<FolderDto[]>();
    mocks.listNotes.mockReturnValueOnce(notes.promise);
    mocks.listFolders.mockReturnValueOnce(folders.promise);
    const { result } = renderHook(() => useSurfaces());

    await emit({ profile: "default", kind: "projects", id: "project" });
    await waitFor(() => expect(mocks.listNotes).toHaveBeenCalled());
    await emit({ profile: "default", kind: "notes", id: note.id, deleted: true });
    await waitFor(() => expect(result.current.state.notes).toEqual([]));
    expect(result.current.tabs[0].nav).toEqual({ view: "notes" });

    await act(async () => {
      notes.resolve({ items: [note], hasMore: false });
      folders.resolve([folder]);
    });
    expect(result.current.state.notes).toEqual([]);
    expect(result.current.state.folders).toEqual([folder]);
  });
});
