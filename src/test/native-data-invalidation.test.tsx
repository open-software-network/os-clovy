import { act, render, renderHook, screen, waitFor } from "@testing-library/react";
import { useReducer, useRef, useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NoteSaveController } from "../app/note-save-controller";
import { createAppDomainActions } from "../app/app-domain-actions";
import type { CreateAppDomainActionsDependencies } from "../app/app-domain-actions-types";
import { createInitialState, notesReducer } from "../app/state/app-state";
import { useNoteProcessingEvents } from "../app/use-note-processing-events";
import { useSessionMetadata } from "../app/use-session-metadata";
import { NoteEditor } from "../components/note-editor/NoteEditor";
import type { ClovyDataChange } from "../lib/clovy-data-events";
import {
  resetCurrentDataPartitionForTests,
  setCurrentDataPartitionName,
} from "../lib/data-partition";
import {
  NOTE_PROCESSING_PROGRESS_EVENT,
  type LiveTranscriptEventDto,
  type NoteDto,
  type NoteProcessingProgressDto,
} from "../lib/tauri";

const mocks = vi.hoisted(() => ({
  listen: vi.fn(),
  getNote: vi.fn(),
  listNotes: vi.fn(),
  listCompletedSessions: vi.fn(),
  listSessionFolders: vi.fn(),
  deleteFolder: vi.fn(),
  assignNoteToFolder: vi.fn(),
  removeNoteFromFolder: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("../lib/tauri", async (original) => ({
  ...(await original<typeof import("../lib/tauri")>()),
  ...mocks,
}));
const listeners = new Set<{
  name: string;
  callback: (event: { payload: ClovyDataChange | NoteProcessingProgressDto }) => void;
}>();
function deferred<T>() {
  let resolve: (value: T) => void = () => {
    throw new Error("Deferred not initialized");
  };
  const promise = new Promise<T>((finish) => {
    resolve = finish;
  });
  return { promise, resolve };
}
async function emit(change: ClovyDataChange) {
  await act(async () => {
    for (const listener of listeners)
      if (listener.name === "clovy-data-changed") listener.callback({ payload: change });
  });
}
const stamp = "2026-11-01T00:00:00Z";
function note(content: string): NoteDto {
  return {
    id: "note",
    title: content,
    preview: content,
    editedContent: content,
    activeTab: "notes",
    processingStatus: "ready",
    createdAt: stamp,
    updatedAt: stamp,
    folderIds: [],
  };
}
function useNotes() {
  const [state, dispatch] = useReducer(notesReducer, {
    ...createInitialState(),
    notes: [note("original")],
    selectedNote: note("original"),
    selectedNoteId: "note",
  });
  const [error, setError] = useState<string | null>(null);
  const [liveTranscriptEvents, setLiveTranscriptEvents] = useState<LiveTranscriptEventDto[]>([]);
  const controller = useRef(
    new NoteSaveController({
      persist: async () => {
        throw new Error("Unexpected save");
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
  return { state, dispatch, controller, error };
}

function actionDependencies(
  overrides: Partial<CreateAppDomainActionsDependencies>,
): CreateAppDomainActionsDependencies {
  return {
    dispatch: vi.fn(),
    noteSaveController: new NoteSaveController({
      persist: async (id, patch) => ({ ...note("original"), id, ...patch }),
    }),
    folderChangesRef: { current: { notes: new Map(), deletedProjects: new Map() } },
    setError: vi.fn(),
    state: createInitialState(),
    ...overrides,
  };
}
beforeEach(() => {
  vi.clearAllMocks();
  listeners.clear();
  resetCurrentDataPartitionForTests();
  mocks.listen.mockImplementation(async (name, callback) => {
    const entry = { name, callback };
    listeners.add(entry);
    return () => listeners.delete(entry);
  });
  mocks.listNotes.mockResolvedValue({ items: [note("new")], hasMore: false });
  mocks.listSessionFolders.mockResolvedValue([]);
  mocks.listCompletedSessions.mockResolvedValue([]);
  mocks.deleteFolder.mockResolvedValue(undefined);
});

describe("native mutation invalidation", () => {
  it.each([
    false,
    true,
  ])("reconciles native list fields after a local removal (content pending: %s)", async (contentPending) => {
    const localRemoval = deferred<NoteDto>();
    const nativeList = deferred<{ items: NoteDto[]; hasMore: boolean }>();
    const old = { ...note("old title"), folderIds: ["p"] };
    const newer = {
      ...note("native title"),
      preview: "native preview",
      processingStatus: "failed" as const,
      folderIds: ["p"],
    };
    mocks.removeNoteFromFolder.mockReturnValueOnce(localRemoval.promise);
    mocks.getNote.mockResolvedValueOnce(newer);
    mocks.listNotes.mockReturnValueOnce(nativeList.promise);
    const { result } = renderHook(() => {
      const [state, dispatch] = useReducer(notesReducer, { ...createInitialState(), notes: [old] });
      const [live, setLive] = useState<LiveTranscriptEventDto[]>([]);
      const [, setError] = useState<string | null>(null);
      const controller = useRef(
        new NoteSaveController({
          debounceMs: 60000,
          persist: async (id, patch) => ({ ...old, id, ...patch }),
        }),
      ).current;
      const folderChangesRef = useRef({
        notes: new Map<string, number>(),
        deletedProjects: new Map<string, number>(),
      });
      useNoteProcessingEvents({
        dispatch,
        noteSaveController: controller,
        folderChangesRef,
        notes: state.notes,
        selectedNote: state.selectedNote,
        liveTranscriptEvents: live,
        setLiveTranscriptEvents: setLive,
        setError,
      });
      const actions = createAppDomainActions(
        actionDependencies({
          state,
          dispatch,
          noteSaveController: controller,
          folderChangesRef,
          setError,
        }),
      );
      return { state, dispatch, controller, actions };
    });
    let removal!: Promise<void>;
    act(() => {
      removal = result.current.actions.handleRemoveNoteFromFolder("note", "p");
    });
    await waitFor(() => expect(mocks.removeNoteFromFolder).toHaveBeenCalledTimes(1));
    await emit({ profile: "default", kind: "notes", id: "note" });
    await waitFor(() => expect(mocks.listNotes).toHaveBeenCalledTimes(1));
    await act(async () => {
      localRemoval.resolve({ ...old, folderIds: [] });
      await removal;
    });
    if (contentPending) {
      act(() => {
        result.current.controller.queue("note", { title: "local title" });
        result.current.dispatch({
          type: "noteUpdated",
          note: { ...old, title: "local title", folderIds: [] },
        });
      });
    }
    await act(async () => nativeList.resolve({ items: [newer], hasMore: false }));
    const row = result.current.state.notes[0];
    expect(row.folderIds).toEqual([]);
    expect(row.title).toBe(contentPending ? "local title" : "native title");
    expect(row.preview).toBe(contentPending ? "old title" : "native preview");
    expect(row.processingStatus).toBe(contentPending ? "ready" : "failed");
  });

  it.each([
    ["remove", true],
    ["move", true],
    ["move", false],
    ["delete", true],
  ] as const)("preserves local %s against an older note read (edit: %s)", async (operation, edit) => {
    const pending = deferred<NoteDto>();
    const initial = { ...note("original"), folderIds: ["p"] };
    const { result } = renderHook(() => {
      const [state, dispatch] = useReducer(notesReducer, {
        ...createInitialState(),
        notes: [initial],
        selectedNote: initial,
        selectedNoteId: "note",
      });
      const [live, setLive] = useState<LiveTranscriptEventDto[]>([]);
      const [, setError] = useState<string | null>(null);
      const controller = useRef(
        new NoteSaveController({
          debounceMs: 60000,
          persist: async (id, patch) => ({ ...note("original"), id, ...patch }),
        }),
      ).current;
      const folderChangesRef = useRef({
        notes: new Map<string, number>(),
        deletedProjects: new Map<string, number>(),
      });
      useNoteProcessingEvents({
        dispatch,
        noteSaveController: controller,
        folderChangesRef,
        notes: state.notes,
        selectedNote: state.selectedNote,
        liveTranscriptEvents: live,
        setLiveTranscriptEvents: setLive,
        setError,
      });
      const actions = createAppDomainActions(
        actionDependencies({
          state,
          dispatch,
          noteSaveController: controller,
          folderChangesRef,
          setError,
        }),
      );
      return { state, dispatch, controller, actions };
    });
    mocks.getNote.mockReturnValueOnce(pending.promise);
    mocks.removeNoteFromFolder.mockResolvedValue({
      ...note(edit ? "local" : "original"),
      folderIds: [],
    });
    mocks.assignNoteToFolder.mockResolvedValue({
      ...note(edit ? "local" : "original"),
      folderIds: ["q"],
    });
    await emit({ profile: "default", kind: "notes" });
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(1));
    if (edit) {
      act(() => {
        result.current.controller.queue("note", { editedContent: "local" });
        result.current.dispatch({
          type: "noteUpdated",
          note: { ...initial, editedContent: "local" },
        });
      });
    }
    await act(async () => {
      if (operation === "remove")
        await result.current.actions.handleRemoveNoteFromFolder("note", "p");
      else if (operation === "move") await result.current.actions.handleSetNoteFolder("note", "q");
      else await result.current.actions.handleDeleteFolder("p");
    });
    await act(async () =>
      pending.resolve({ ...initial, editedContent: edit ? "original" : "native body" }),
    );
    await waitFor(() =>
      expect(result.current.state.selectedNote?.folderIds).toEqual(
        operation === "move" ? ["q"] : [],
      ),
    );
    expect(result.current.state.selectedNote?.editedContent).toBe(edit ? "local" : "native body");
  });
  it("applies the newest assignment when an older queued native update is superseded", async () => {
    type Folders = Record<string, string[]>;
    const updates: React.SetStateAction<Folders>[] = [];
    mocks.listSessionFolders.mockResolvedValue([{ sessionId: "a", folderId: "project" }]);
    const { result } = renderHook(() => {
      const [folders, setFolders] = useState<Folders>({});
      useSessionMetadata({
        appBlocked: false,
        bootstrapped: false,
        currentDataPartitionName: "default",
        sessionFolderEditsRef: useRef(new Map()),
        folderChangesRef: useRef({ notes: new Map(), deletedProjects: new Map() }),
        sessionCompletionTouchedRef: useRef(new Set()),
        setCompletedSessions: () => {},
        setSessionFolders: (update) => {
          updates.push(update);
        },
        setError: () => {},
      });
      return { folders, setFolders };
    });
    await emit({ profile: "default", kind: "projects" });
    await waitFor(() => expect(updates).toHaveLength(1));
    await emit({ profile: "default", kind: "projects" });
    await waitFor(() => expect(updates).toHaveLength(2));
    act(() => {
      for (const update of updates) result.current.setFolders(update);
    });
    expect(result.current.folders).toEqual({ a: ["project"] });
  });
  it.each([
    "projects",
    "conversations",
  ] as const)("lets a later %s removal win over an unrendered native update while preserving a local edit", async (kind) => {
    type Folders = Record<string, string[]>;
    let pending: Folders = {};
    let updates = 0;
    const edits = { current: new Map<string, number>() };
    const second = deferred<Array<{ sessionId: string; folderId: string }>>();
    mocks.listSessionFolders
      .mockResolvedValueOnce([{ sessionId: "a", folderId: "project" }])
      .mockReturnValueOnce(second.promise);
    const { result } = renderHook(() => {
      const [folders, setFolders] = useState<Folders>({});
      const touched = useRef(new Set<string>());
      useSessionMetadata({
        appBlocked: false,
        bootstrapped: false,
        currentDataPartitionName: "default",
        sessionFolderEditsRef: edits,
        folderChangesRef: useRef({ notes: new Map(), deletedProjects: new Map() }),
        sessionCompletionTouchedRef: touched,
        setCompletedSessions: () => {},
        setSessionFolders: (update) => {
          pending = typeof update === "function" ? update(pending) : update;
          updates++;
        },
        setError: () => {},
      });
      return { folders, setFolders };
    });
    await emit({ profile: "default", kind });
    await waitFor(() => expect(updates).toBe(1));
    await emit({ profile: "default", kind });
    pending = { ...pending, local: ["local-project"] };
    edits.current.set("local", 1);
    await act(async () => second.resolve([]));
    await waitFor(() => expect(updates).toBe(2));
    act(() => result.current.setFolders(pending));
    expect(result.current.folders).toEqual({ local: ["local-project"] });
  });
  it("reloads both session metadata snapshots after a partition change during startup", async () => {
    const oldFolders = deferred<Array<{ sessionId: string; folderId: string }>>();
    const oldCompleted = deferred<Array<{ sessionId: string; completedAt: string }>>();
    mocks.listSessionFolders
      .mockReturnValueOnce(oldFolders.promise)
      .mockResolvedValueOnce([{ sessionId: "new", folderId: "project" }]);
    mocks.listCompletedSessions
      .mockReturnValueOnce(oldCompleted.promise)
      .mockResolvedValueOnce([{ sessionId: "new", completedAt: stamp }]);
    const { result, rerender } = renderHook(
      ({ partition }) => {
        const [completed, setCompletedSessions] = useState<Record<string, string>>({});
        const [folders, setSessionFolders] = useState<Record<string, string[]>>({});
        const touched = useRef(new Set<string>());
        useSessionMetadata({
          appBlocked: false,
          bootstrapped: true,
          currentDataPartitionName: partition,
          sessionFolderEditsRef: useRef(new Map<string, number>()),
          folderChangesRef: useRef({ notes: new Map(), deletedProjects: new Map() }),
          sessionCompletionTouchedRef: touched,
          setCompletedSessions,
          setSessionFolders,
          setError: () => {},
        });
        return { completed, folders };
      },
      { initialProps: { partition: "default" } },
    );
    await waitFor(() => expect(mocks.listCompletedSessions).toHaveBeenCalledTimes(1));
    act(() => setCurrentDataPartitionName("other"));
    rerender({ partition: "other" });
    await waitFor(() => expect(result.current.completed).toEqual({ new: stamp }));
    expect(result.current.folders).toEqual({ new: ["project"] });
    await act(async () => {
      oldFolders.resolve([]);
      oldCompleted.resolve([]);
    });
    expect(result.current.completed).toEqual({ new: stamp });
    expect(result.current.folders).toEqual({ new: ["project"] });
  });
  it("updates the open editor project chip on generic assignment and removal", async () => {
    function Surface() {
      const { state } = useNotes();
      const noop = () => {};
      return (
        state.selectedNote && (
          <NoteEditor
            note={state.selectedNote}
            folders={[
              {
                id: "project",
                name: "Synthetic project",
                memoryDisabled: false,
                createdAt: stamp,
                updatedAt: stamp,
              },
            ]}
            sourceMode="microphoneOnly"
            microphoneBlocked={false}
            onTitleChange={noop}
            onContentChange={noop}
            onFlushNote={noop}
            onSourceModeChange={noop}
            onEnableSystemAudio={noop}
            onEnableMicrophone={noop}
            onStartRecording={noop}
            onPauseRecording={noop}
            onResumeRecording={noop}
            onFinishRecording={noop}
            onRetry={noop}
            onRecoverRecording={noop}
            onDiscardRecording={noop}
            onAssignFolder={noop}
            onRemoveFolder={noop}
            onCreateAndAssignFolder={noop}
            onTabChange={noop}
          />
        )
      );
    }
    render(<Surface />);
    mocks.getNote.mockResolvedValueOnce({ ...note("original"), folderIds: ["project"] });
    await emit({ profile: "default", kind: "notes" });
    await screen.findByRole("button", { name: "Synthetic project" });
    mocks.getNote.mockResolvedValueOnce({ ...note("original"), folderIds: [] });
    await emit({ profile: "default", kind: "notes" });
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Synthetic project" })).not.toBeInTheDocument(),
    );
  });
  it("reconciles the open note after generic project association changes without overwriting a concurrent edit", async () => {
    const { result } = renderHook(useNotes);
    mocks.getNote.mockResolvedValueOnce({ ...note("original"), folderIds: ["project"] });
    await emit({ profile: "default", kind: "notes" });
    await waitFor(() => expect(result.current.state.selectedNote?.folderIds).toEqual(["project"]));
    const pending = deferred<NoteDto>();
    mocks.getNote.mockReturnValueOnce(pending.promise);
    await emit({ profile: "default", kind: "notes" });
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(2));
    act(() => {
      result.current.controller.queue("note", { title: "local", editedContent: "local" });
      result.current.dispatch({
        type: "noteUpdated",
        note: { ...note("local"), folderIds: ["project"] },
      });
    });
    await act(async () => pending.resolve({ ...note("old"), folderIds: [] }));
    expect(result.current.state.selectedNote?.editedContent).toBe("local");
    await waitFor(() => expect(result.current.state.selectedNote?.folderIds).toEqual([]));
  });

  it("keeps native completion and assignment refreshes when initial snapshots resolve late", async () => {
    const completions = deferred<Array<{ sessionId: string; completedAt: string }>>();
    const assignments = deferred<Array<{ sessionId: string; folderId: string }>>();
    const nativeAssignments = deferred<Array<{ sessionId: string; folderId: string }>>();
    mocks.listCompletedSessions
      .mockReturnValueOnce(completions.promise)
      .mockResolvedValue([{ sessionId: "a", completedAt: stamp }]);
    mocks.listSessionFolders
      .mockReturnValueOnce(assignments.promise)
      .mockReturnValueOnce(nativeAssignments.promise);
    const { result } = renderHook(() => {
      const [completed, setCompletedSessions] = useState<Record<string, string>>({});
      const [folders, setSessionFolders] = useState<Record<string, string[]>>({});
      const [, setError] = useState<string | null>(null);
      const touched = useRef(new Set<string>());
      const edits = useRef(new Map<string, number>());
      useSessionMetadata({
        appBlocked: false,
        bootstrapped: true,
        currentDataPartitionName: "default",
        sessionFolderEditsRef: edits,
        folderChangesRef: useRef({ notes: new Map(), deletedProjects: new Map() }),
        sessionCompletionTouchedRef: touched,
        setCompletedSessions,
        setSessionFolders,
        setError,
      });
      return { completed, folders, setCompletedSessions, setSessionFolders, touched, edits };
    });
    await waitFor(() => expect(mocks.listCompletedSessions).toHaveBeenCalledTimes(1));
    await emit({ profile: "default", kind: "conversations", id: "a" });
    act(() => {
      result.current.touched.current.add("local");
      result.current.edits.current.set("local", 1);
      result.current.setCompletedSessions({ local: stamp });
      result.current.setSessionFolders({ local: ["local-project"] });
    });
    await act(async () => nativeAssignments.resolve([{ sessionId: "a", folderId: "project" }]));
    await waitFor(() => expect(result.current.completed).toEqual({ a: stamp, local: stamp }));
    expect(result.current.folders).toEqual({ a: ["project"], local: ["local-project"] });
    await act(async () => {
      completions.resolve([]);
      assignments.resolve([]);
    });
    expect(result.current.completed).toEqual({ a: stamp, local: stamp });
    expect(result.current.folders).toEqual({ a: ["project"], local: ["local-project"] });
  });
  it("does not resurrect deleted notes from a delayed terminal hydration or its retry", async () => {
    const old = deferred<NoteDto>();
    mocks.getNote.mockReturnValueOnce(old.promise);
    const { result } = renderHook(useNotes);
    const progress = {
      noteId: "note",
      recordingSessionId: "take",
      stage: "done",
      revision: stamp,
      processingStatus: "ready",
    } as NoteProcessingProgressDto;
    await act(async () => {
      for (const listener of listeners)
        if (listener.name === NOTE_PROCESSING_PROGRESS_EVENT)
          listener.callback({ payload: progress });
    });
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(1));
    mocks.listNotes.mockResolvedValue({ items: [], hasMore: false });
    await emit({ profile: "default", kind: "notes", id: "note", deleted: true });
    await act(async () => {
      old.resolve(note("stale completed result"));
    });
    expect(result.current.state.notes).toEqual([]);
    expect(result.current.state.selectedNote).toBeUndefined();
    await act(async () => {
      for (const listener of listeners)
        if (listener.name === NOTE_PROCESSING_PROGRESS_EVENT)
          listener.callback({ payload: progress });
      window.dispatchEvent(new Event("focus"));
    });
    expect(mocks.getNote).toHaveBeenCalledTimes(1);
    expect(result.current.error).toBeNull();
  });
  it("does not resurrect a deleted note from an older hydration response", async () => {
    const old = deferred<NoteDto>();
    mocks.getNote.mockReturnValueOnce(old.promise);
    const { result } = renderHook(useNotes);
    await emit({ profile: "default", kind: "notes", id: "note" });
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(1));
    mocks.listNotes.mockResolvedValue({ items: [], hasMore: false });
    await emit({ profile: "default", kind: "notes", id: "note", deleted: true });
    expect(result.current.state.selectedNote).toBeUndefined();
    await act(async () => {
      old.resolve(note("stale"));
    });
    expect(result.current.state.notes).toEqual([]);
    expect(result.current.state.selectedNote).toBeUndefined();
  });

  it("keeps the newest response when two native edits finish out of order", async () => {
    const old = deferred<NoteDto>();
    const latest = deferred<NoteDto>();
    mocks.getNote.mockReturnValueOnce(old.promise).mockReturnValueOnce(latest.promise);
    const { result } = renderHook(useNotes);
    await emit({ profile: "default", kind: "notes", id: "note" });
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(1));
    await emit({ profile: "default", kind: "notes", id: "note" });
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(2));
    await act(async () => {
      latest.resolve(note("new"));
    });
    expect(result.current.state.selectedNote?.editedContent).toBe("new");
    await act(async () => {
      old.resolve(note("old"));
    });
    expect(result.current.state.selectedNote?.editedContent).toBe("new");
  });

  it("preserves an optimistic edit queued while the native snapshot is loading", async () => {
    const loaded = deferred<NoteDto>();
    mocks.getNote.mockReturnValueOnce(loaded.promise);
    const { result } = renderHook(useNotes);
    await emit({ profile: "default", kind: "notes", id: "note" });
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(1));
    act(() => {
      result.current.controller.queue("note", { title: "local", editedContent: "local" });
      result.current.dispatch({ type: "noteUpdated", note: note("local") });
    });
    await act(async () => {
      loaded.resolve(note("old database snapshot"));
    });
    expect(result.current.state.selectedNote?.editedContent).toBe("local");
    expect(result.current.state.notes[0].title).toBe("local");
    result.current.controller.discard("note");
  });

  it("reconciles every conversation invalidated during an overlapping archive reload", async () => {
    const first = deferred<Array<{ sessionId: string; completedAt: string }>>();
    mocks.listCompletedSessions.mockReturnValueOnce(first.promise).mockResolvedValueOnce([
      { sessionId: "a", completedAt: stamp },
      { sessionId: "b", completedAt: stamp },
    ]);
    const { result } = renderHook(() => {
      const [completed, setCompletedSessions] = useState<Record<string, string>>({});
      const [, setSessionFolders] = useState<Record<string, string[]>>({});
      const [, setError] = useState<string | null>(null);
      const touched = useRef(new Set<string>());
      useSessionMetadata({
        appBlocked: false,
        bootstrapped: false,
        currentDataPartitionName: "default",
        sessionFolderEditsRef: useRef(new Map<string, number>()),
        folderChangesRef: useRef({ notes: new Map(), deletedProjects: new Map() }),
        sessionCompletionTouchedRef: touched,
        setCompletedSessions,
        setSessionFolders,
        setError,
      });
      return completed;
    });
    await emit({ profile: "default", kind: "conversations", id: "a" });
    await waitFor(() => expect(mocks.listCompletedSessions).toHaveBeenCalledTimes(1));
    await emit({ profile: "default", kind: "conversations", id: "b" });
    await waitFor(() => expect(result.current).toEqual({ a: stamp, b: stamp }));
    await act(async () => {
      first.resolve([]);
    });
    expect(result.current).toEqual({ a: stamp, b: stamp });
  });
});
