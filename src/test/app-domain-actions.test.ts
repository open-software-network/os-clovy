import { beforeEach, describe, expect, it, vi } from "vitest";
import { createAppDomainActions } from "../app/app-domain-actions";
import type { CreateAppDomainActionsDependencies } from "../app/app-domain-actions-types";
import type { NoteSaveController } from "../app/note-save-controller";
import { createInitialState } from "../app/state/app-state";
import type { NoteDto } from "../lib/tauri";

const tauriMocks = vi.hoisted(() => ({
  assignNoteToFolder: vi.fn(),
  assignSessionToFolder: vi.fn(),
  deleteFolder: vi.fn(),
  removeNoteFromFolder: vi.fn(),
  removeSessionFromFolder: vi.fn(),
  setSessionCompleted: vi.fn(),
}));

vi.mock("../lib/tauri", () => tauriMocks);

const now = "2026-07-23T12:00:00Z";

function note(folderIds: string[]): NoteDto {
  return {
    id: "note-1",
    title: "Draft",
    preview: "Unsaved body",
    processingStatus: "draft",
    folderIds,
    createdAt: now,
    updatedAt: now,
    activeTab: "notes",
  };
}

function dependencies(flush: ReturnType<typeof vi.fn>): CreateAppDomainActionsDependencies {
  const currentNote = note(["folder-old"]);
  return {
    dispatch: vi.fn(),
    noteSaveController: { flush } as unknown as NoteSaveController,
    folderChangesRef: { current: { notes: new Map(), deletedProjects: new Map() } },
    setError: vi.fn(),
    state: {
      ...createInitialState(),
      notes: [currentNote],
      selectedNoteId: currentNote.id,
      selectedNote: currentNote,
    },
  };
}

describe("app note folder actions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriMocks.removeNoteFromFolder.mockResolvedValue(note([]));
    tauriMocks.assignNoteToFolder.mockResolvedValue(note(["folder-new"]));
  });

  it("flushes a note before moving it to another folder", async () => {
    const flush = vi.fn().mockResolvedValue(undefined);
    const actions = createAppDomainActions(dependencies(flush));

    await actions.handleSetNoteFolder("note-1", "folder-new", { rethrow: true });

    expect(flush).toHaveBeenCalledWith("note-1");
    expect(tauriMocks.removeNoteFromFolder).toHaveBeenCalledWith("note-1", "folder-old");
    expect(tauriMocks.assignNoteToFolder).toHaveBeenCalledWith("note-1", "folder-new");
    expect(flush.mock.invocationCallOrder[0]).toBeLessThan(
      tauriMocks.removeNoteFromFolder.mock.invocationCallOrder[0],
    );
  });

  it("flushes a note before removing its folder assignment", async () => {
    const flush = vi.fn().mockResolvedValue(undefined);
    const actions = createAppDomainActions(dependencies(flush));

    await actions.handleRemoveNoteFromFolder("note-1", "folder-old", { rethrow: true });

    expect(flush).toHaveBeenCalledWith("note-1");
    expect(tauriMocks.removeNoteFromFolder).toHaveBeenCalledWith("note-1", "folder-old");
    expect(flush.mock.invocationCallOrder[0]).toBeLessThan(
      tauriMocks.removeNoteFromFolder.mock.invocationCallOrder[0],
    );
  });
});
