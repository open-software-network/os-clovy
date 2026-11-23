import { describe, expect, it } from "vitest";
import { createInitialState, notesReducer } from "../app/state/app-state";
import type { BootstrapResponse, NoteDto, RecordingStatusDto } from "../lib/tauri";

const now = "2026-05-19T10:00:00Z";

function note(overrides: Partial<NoteDto> = {}): NoteDto {
  return {
    id: "note-1",
    title: "",
    preview: "",
    processingStatus: "draft",
    folderIds: [],
    createdAt: now,
    updatedAt: now,
    activeTab: "notes",
    ...overrides,
  };
}

describe("notesReducer", () => {
  it("fences confirmed deletions across stale list and detail producers while retaining other notes", () => {
    const deleted = note();
    const kept = note({ id: "kept", title: "Keep me" });
    let state = notesReducer(createInitialState(), { type: "notesLoaded", notes: [deleted, kept] });
    state = notesReducer(state, { type: "noteDeleted", noteId: deleted.id });
    state = notesReducer(state, { type: "notesLoaded", notes: [deleted, kept] });
    state = notesReducer(state, { type: "notesInvalidated", notes: [deleted, kept] });
    state = notesReducer(state, { type: "noteLoaded", note: deleted });
    state = notesReducer(state, { type: "noteUpdated", note: deleted });
    state = notesReducer(state, { type: "noteProcessingUpdated", note: deleted });
    expect(state.notes).toEqual([kept]);
    expect(state.selectedNoteId).toBe(kept.id);
    state = notesReducer(state, {
      type: "noteLoaded",
      note: { ...kept, title: "Fresh kept title" },
    });
    expect(state.selectedNote?.title).toBe("Fresh kept title");
  });
  it("clears a deleted selection even when a later batch refresh concerns another note", () => {
    const selected = note({ id: "deleted-first" });
    const initial = notesReducer(createInitialState(), { type: "noteLoaded", note: selected });
    const removed = notesReducer(initial, { type: "noteDeleted", noteId: selected.id });
    const refreshed = notesReducer(removed, {
      type: "notesInvalidated",
      notes: [],
      deletedId: "deleted-last",
    });
    expect(refreshed.selectedNoteId).toBeUndefined();
    expect(refreshed.selectedNote).toBeUndefined();
    expect(refreshed.notes).toHaveLength(0);
  });
  it("refreshes the note list without losing selection, and clears a deleted selection", () => {
    const selected = note({ id: "note-1", title: "Current" });
    const initial = notesReducer(createInitialState(), { type: "noteLoaded", note: selected });
    const refreshed = notesReducer(initial, {
      type: "notesInvalidated",
      notes: [selected, note({ id: "note-2" })],
    });
    expect(refreshed.selectedNote).toEqual(selected);
    const deleted = notesReducer(refreshed, {
      type: "notesInvalidated",
      notes: [note({ id: "note-2" })],
      deletedId: "note-1",
    });
    expect(deleted.selectedNote).toBeUndefined();
    expect(deleted.selectedNoteId).toBeUndefined();
  });
  it("loads bootstrap data and selects the first note", () => {
    const payload: BootstrapResponse = {
      folders: [
        { id: "folder-1", name: "Ideas", memoryDisabled: false, createdAt: now, updatedAt: now },
      ],
      notes: [note({ id: "note-2", title: "Second" }), note({ id: "note-1", title: "First" })],
      activeRecoveries: [],
    };

    const state = notesReducer(createInitialState(), {
      type: "bootstrapLoaded",
      payload,
    });

    expect(state.folders).toHaveLength(1);
    expect(state.notes.map((item) => item.id)).toEqual(["note-2", "note-1"]);
    expect(state.selectedNoteId).toBe("note-2");
  });

  it("upserts a folder when a concurrent import returns an existing folder", () => {
    const folder = {
      id: "folder-1",
      name: "Project",
      memoryDisabled: false,
      localPath: "/Users/alex/code/project",
      createdAt: now,
      updatedAt: now,
    };
    const initial = { ...createInitialState(), folders: [folder] };

    const state = notesReducer(initial, {
      type: "folderCreated",
      folder: { ...folder, name: "Updated project" },
    });

    expect(state.folders).toHaveLength(1);
    expect(state.folders[0].name).toBe("Updated project");
  });

  it("updates the selected note after autosave", () => {
    const initial = notesReducer(createInitialState(), {
      type: "noteLoaded",
      note: note({ id: "note-1", title: "Draft" }),
    });

    const state = notesReducer(initial, {
      type: "noteUpdated",
      note: note({
        id: "note-1",
        title: "Edited",
        editedContent: "Clean notes",
      }),
    });

    expect(state.notes[0].title).toBe("Edited");
    expect(state.selectedNote?.editedContent).toBe("Clean notes");
  });

  it("ignores nullable database fields in a note patch", () => {
    const initial = notesReducer(createInitialState(), {
      type: "noteLoaded",
      note: note({
        id: "note-1",
        title: "Existing title",
        editedContent: "Existing edit",
        activeTab: "transcription",
      }),
    });

    const state = notesReducer(initial, {
      type: "notePatched",
      noteId: "note-1",
      patch: {
        title: "Patched title",
        preview: "Patched preview",
        editedContent: null,
        activeTab: null,
        updatedAt: "2026-07-23T12:00:00Z",
      } as never,
    });

    expect(state.selectedNote?.title).toBe("Patched title");
    expect(state.selectedNote?.editedContent).toBe("Existing edit");
    expect(state.selectedNote?.activeTab).toBe("transcription");
  });

  it("keeps optimistic transcribing status when a stale validating note arrives", () => {
    const initial = notesReducer(createInitialState(), {
      type: "noteLoaded",
      note: note({
        id: "note-1",
        processingStatus: "transcribing",
      }),
    });

    const state = notesReducer(initial, {
      type: "noteUpdated",
      note: note({
        id: "note-1",
        title: "Server title",
        processingStatus: "validating",
      }),
    });

    expect(state.selectedNote?.title).toBe("Server title");
    expect(state.selectedNote?.processingStatus).toBe("transcribing");
    expect(state.notes[0].processingStatus).toBe("transcribing");
  });

  it("does not move generating backward when polling responses finish out of order", () => {
    const initial = notesReducer(createInitialState(), {
      type: "noteLoaded",
      note: note({
        id: "note-1",
        processingStatus: "generating",
      }),
    });

    const state = notesReducer(initial, {
      type: "noteUpdated",
      note: note({
        id: "note-1",
        processingStatus: "transcribing",
      }),
    });

    expect(state.selectedNote?.processingStatus).toBe("generating");
    expect(state.notes[0].processingStatus).toBe("generating");
  });

  it("accepts terminal processing statuses after optimistic transcribing", () => {
    const initial = notesReducer(createInitialState(), {
      type: "noteLoaded",
      note: note({
        id: "note-1",
        processingStatus: "transcribing",
      }),
    });

    const state = notesReducer(initial, {
      type: "noteUpdated",
      note: note({
        id: "note-1",
        processingStatus: "failed",
        lastError: "Transcription failed",
      }),
    });

    expect(state.selectedNote?.processingStatus).toBe("failed");
    expect(state.selectedNote?.lastError).toBe("Transcription failed");
  });

  it("lets an authoritative command restart processing on a failed note", () => {
    // Retry returns the note in an active status; without the bypass the
    // terminal guard would swallow it and the failure banner would never
    // clear even though the backend is reprocessing.
    const initial = notesReducer(createInitialState(), {
      type: "noteLoaded",
      note: note({
        id: "note-1",
        processingStatus: "failed",
        lastError: "Transcription failed",
      }),
    });

    const state = notesReducer(initial, {
      type: "noteProcessingUpdated",
      note: note({
        id: "note-1",
        processingStatus: "transcribing",
      }),
    });

    expect(state.selectedNote?.processingStatus).toBe("transcribing");
    expect(state.notes[0].processingStatus).toBe("transcribing");
  });

  it("lets a new take restart processing on a ready note", () => {
    // Stacking another recording on an already-ready note: the finish flow
    // marks it transcribing again so the shimmer shows and polling resumes.
    const initial = notesReducer(createInitialState(), {
      type: "noteLoaded",
      note: note({
        id: "note-1",
        processingStatus: "ready",
        generatedContent: "Finished notes",
      }),
    });

    const state = notesReducer(initial, {
      type: "noteProcessingUpdated",
      note: note({
        id: "note-1",
        processingStatus: "transcribing",
        generatedContent: "Finished notes",
      }),
    });

    expect(state.selectedNote?.processingStatus).toBe("transcribing");
    expect(state.notes[0].processingStatus).toBe("transcribing");
  });

  it("does not move a terminal note backward after stale active polling", () => {
    const initial = notesReducer(createInitialState(), {
      type: "noteLoaded",
      note: note({
        id: "note-1",
        processingStatus: "ready",
        generatedContent: "Finished notes",
      }),
    });

    const state = notesReducer(initial, {
      type: "noteUpdated",
      note: note({
        id: "note-1",
        processingStatus: "transcribing",
      }),
    });

    expect(state.selectedNote?.processingStatus).toBe("ready");
    expect(state.notes[0].processingStatus).toBe("ready");
  });

  it("does not let stale active polling overwrite a ready title", () => {
    const initial = notesReducer(createInitialState(), {
      type: "noteLoaded",
      note: note({
        id: "note-1",
        title: "Budget review, Hiring plan, and Product launch",
        processingStatus: "ready",
        generatedContent: "Finished notes",
      }),
    });

    const state = notesReducer(initial, {
      type: "noteUpdated",
      note: note({
        id: "note-1",
        title: "New note",
        processingStatus: "transcribing",
        generatedContent: "",
      }),
    });

    expect(state.selectedNote?.title).toBe("Budget review, Hiring plan, and Product launch");
    expect(state.selectedNote?.generatedContent).toBe("Finished notes");
    expect(state.selectedNote?.processingStatus).toBe("ready");
    expect(state.notes[0].title).toBe("Budget review, Hiring plan, and Product launch");
  });

  it("keeps source transcript rows on authoritative processing updates", () => {
    const initial = notesReducer(createInitialState(), {
      type: "noteLoaded",
      note: note({
        id: "note-1",
        processingStatus: "generating",
      }),
    });

    const state = notesReducer(initial, {
      type: "noteProcessingUpdated",
      note: note({
        id: "note-1",
        processingStatus: "ready",
        sourceTranscripts: [
          {
            id: "turn-1",
            text: "System first",
            source: "system",
            startMs: 1000,
            endMs: 2000,
            turnIndex: 0,
            status: "succeeded",
          },
          {
            id: "turn-2",
            text: "Microphone second",
            source: "microphone",
            startMs: 2500,
            endMs: 3500,
            turnIndex: 1,
            status: "succeeded",
          },
        ],
      }),
    });

    expect(state.selectedNote?.processingStatus).toBe("ready");
    expect(state.selectedNote?.sourceTranscripts?.map((turn) => turn.text)).toEqual([
      "System first",
      "Microphone second",
    ]);
    expect(state.notes[0].processingStatus).toBe("ready");
  });

  it("tracks recording status transitions without changing selected note", () => {
    const initial = notesReducer(createInitialState(), {
      type: "noteLoaded",
      note: note({ id: "note-1" }),
    });
    const status: RecordingStatusDto = {
      sessionId: "session-1",
      state: "recording",
      elapsedMs: 1250,
      level: { peak: 0.4, rms: 0.2, recentPeaks: [0.1, 0.4] },
      silenceWarning: false,
      bytesWritten: 4096,
    };

    const state = notesReducer(initial, {
      type: "recordingStatusChanged",
      status,
    });

    expect(state.selectedNoteId).toBe("note-1");
    expect(state.recordingStatus).toEqual(status);
  });

  it("renames and deletes folders, keeping notes consistent", () => {
    const initial = notesReducer(createInitialState(), {
      type: "bootstrapLoaded",
      payload: {
        folders: [
          { id: "folder-1", name: "Inbox", memoryDisabled: false, createdAt: now, updatedAt: now },
          {
            id: "folder-2",
            name: "Archive",
            memoryDisabled: false,
            createdAt: now,
            updatedAt: now,
          },
        ],
        notes: [
          { ...note({ id: "note-1", title: "A" }), folderIds: ["folder-1"] },
          {
            ...note({ id: "note-2", title: "B" }),
            folderIds: ["folder-1", "folder-2"],
          },
        ],
        activeRecoveries: [],
      },
    });

    expect(initial.folders.map((folder) => folder.name)).toEqual(["Archive", "Inbox"]);

    const renamed = notesReducer(initial, {
      type: "folderRenamed",
      folder: {
        id: "folder-1",
        name: "Triage",
        memoryDisabled: false,
        createdAt: now,
        updatedAt: now,
      },
    });
    expect(renamed.folders.map((folder) => folder.name)).toEqual(["Archive", "Triage"]);

    const updated = notesReducer(renamed, {
      type: "folderUpdated",
      folder: {
        id: "folder-1",
        name: "Triage",
        instructions: "Keep answers concise",
        memoryDisabled: true,
        createdAt: now,
        updatedAt: now,
      },
    });
    expect(updated.folders.find((folder) => folder.id === "folder-1")).toEqual(
      expect.objectContaining({ instructions: "Keep answers concise", memoryDisabled: true }),
    );

    const deleted = notesReducer(updated, {
      type: "folderDeleted",
      folderId: "folder-1",
    });
    expect(deleted.folders.map((folder) => folder.id)).toEqual(["folder-2"]);
    expect(deleted.notes.find((item) => item.id === "note-2")?.folderIds).toEqual(["folder-2"]);
    expect(deleted.notes.find((item) => item.id === "note-1")?.folderIds).toEqual([]);
  });

  it("clears recording status after finish processing starts", () => {
    const status: RecordingStatusDto = {
      sessionId: "session-1",
      state: "recording",
      elapsedMs: 1250,
      level: { peak: 0.4, rms: 0.2, recentPeaks: [0.1, 0.4] },
      silenceWarning: false,
      bytesWritten: 4096,
    };
    const recording = notesReducer(createInitialState(), {
      type: "recordingStatusChanged",
      status,
    });

    const cleared = notesReducer(recording, { type: "recordingStatusCleared" });

    expect(cleared.recordingStatus).toBeUndefined();
  });

  it("clears the recorder when the backend lost the polled session", () => {
    const status: RecordingStatusDto = {
      sessionId: "session-1",
      state: "recording",
      elapsedMs: 1250,
      level: { peak: 0.4, rms: 0.2, recentPeaks: [0.1, 0.4] },
      silenceWarning: false,
      bytesWritten: 4096,
    };
    const recording = notesReducer(createInitialState(), {
      type: "recordingStatusChanged",
      status,
    });

    const lost = notesReducer(recording, {
      type: "recordingSessionLost",
      sessionId: "session-1",
    });

    expect(lost.recordingStatus).toBeUndefined();
  });

  it("ignores a lost-session signal for a superseded recording", () => {
    const status: RecordingStatusDto = {
      sessionId: "session-2",
      state: "recording",
      elapsedMs: 200,
      level: { peak: 0.4, rms: 0.2, recentPeaks: [0.1, 0.4] },
      silenceWarning: false,
      bytesWritten: 1024,
    };
    const recording = notesReducer(createInitialState(), {
      type: "recordingStatusChanged",
      status,
    });

    // A stale poll for the previous session must not tear down the bar for
    // the recording that replaced it.
    const state = notesReducer(recording, {
      type: "recordingSessionLost",
      sessionId: "session-1",
    });

    expect(state.recordingStatus).toEqual(status);
  });
});
