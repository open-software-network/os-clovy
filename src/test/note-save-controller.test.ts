import { afterEach, describe, expect, it, vi } from "vitest";
import {
  NOTE_SAVE_DEBOUNCE_MS,
  NOTE_SAVE_MAX_RETRIES,
  NoteSaveController,
} from "../app/note-save-controller";
import type { NoteEditablePatch, NotePatchDto } from "../lib/tauri";

function persistedPatch(noteId: string, patch: NoteEditablePatch): NotePatchDto {
  return {
    id: noteId,
    title: patch.title ?? "Existing title",
    preview: patch.editedContent ?? patch.title ?? "Existing preview",
    editedContent: patch.editedContent,
    activeTab: patch.activeTab ?? "notes",
    updatedAt: "2026-07-23T10:00:00.000Z",
  };
}

describe("NoteSaveController", () => {
  it.each([
    "success",
    "error",
  ] as const)("ignores discarded in-flight %s without retry or shutdown blockage", async (outcome) => {
    let resolveOld!: (patch: NotePatchDto) => void;
    let rejectOld!: (error: Error) => void;
    const onPersisted = vi.fn();
    const onError = vi.fn();
    const persist = vi.fn((id: string, patch: NoteEditablePatch) =>
      id === "deleted"
        ? new Promise<NotePatchDto>((resolve, reject) => {
            resolveOld = resolve;
            rejectOld = reject;
          })
        : Promise.resolve(persistedPatch(id, patch)),
    );
    const controller = new NoteSaveController({ persist, onPersisted, onError });
    controller.queue("deleted", { editedContent: "Old body" });
    const oldSave = controller.flush("deleted").catch(() => undefined);
    controller.discard("deleted");
    controller.queue("kept", { editedContent: "Save this note" });
    await controller.flushAll();
    expect(controller.hasPending()).toBe(false);
    if (outcome === "success") resolveOld(persistedPatch("deleted", { editedContent: "Old body" }));
    else rejectOld(new Error("RowNotFound"));
    await oldSave;
    await controller.flushAll();
    expect(onPersisted.mock.calls.map(([patch]) => patch.id)).toEqual(["kept"]);
    expect(onError).not.toHaveBeenCalled();
    expect(controller.hasPending()).toBe(false);
    expect(persist.mock.calls.map(([id]) => id)).toEqual(["deleted", "kept"]);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("debounces title and content edits into one note patch", async () => {
    vi.useFakeTimers();
    const persist = vi.fn(async (noteId: string, patch: NoteEditablePatch) =>
      persistedPatch(noteId, patch),
    );
    const controller = new NoteSaveController({ persist });

    controller.queue("note-1", { title: "First" });
    controller.queue("note-1", { title: "Final", editedContent: "Body" });

    await vi.advanceTimersByTimeAsync(NOTE_SAVE_DEBOUNCE_MS - 1);
    expect(persist).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1);
    expect(persist).toHaveBeenCalledTimes(1);
    expect(persist).toHaveBeenCalledWith("note-1", {
      title: "Final",
      editedContent: "Body",
    });
  });

  it("flushes the current note immediately for blur or navigation", async () => {
    vi.useFakeTimers();
    const persist = vi.fn(async (noteId: string, patch: NoteEditablePatch) =>
      persistedPatch(noteId, patch),
    );
    const controller = new NoteSaveController({ persist });

    controller.queue("note-1", { title: "Saved before leaving" });
    await controller.flush("note-1");

    expect(persist).toHaveBeenCalledTimes(1);
    expect(persist).toHaveBeenCalledWith("note-1", {
      title: "Saved before leaving",
    });
    await vi.advanceTimersByTimeAsync(NOTE_SAVE_DEBOUNCE_MS);
    expect(persist).toHaveBeenCalledTimes(1);
  });

  it("flushes every pending note before app shutdown", async () => {
    vi.useFakeTimers();
    const persist = vi.fn(async (noteId: string, patch: NoteEditablePatch) =>
      persistedPatch(noteId, patch),
    );
    const controller = new NoteSaveController({ persist });

    controller.queue("note-1", { title: "First note" });
    controller.queue("note-2", { editedContent: "Second note body" });
    await controller.flushAll();

    expect(persist).toHaveBeenCalledTimes(2);
    expect(persist).toHaveBeenCalledWith("note-1", { title: "First note" });
    expect(persist).toHaveBeenCalledWith("note-2", {
      editedContent: "Second note body",
    });
    expect(controller.hasPending()).toBe(false);
  });

  it("drains a patch queued after the shutdown snapshot", async () => {
    let finishFirst: ((patch: NotePatchDto) => void) | undefined;
    const persist = vi
      .fn<(noteId: string, patch: NoteEditablePatch) => Promise<NotePatchDto>>()
      .mockImplementationOnce(
        (_noteId, patch) =>
          new Promise((resolve) => {
            finishFirst = resolve;
            expect(patch).toEqual({ title: "First note" });
          }),
      )
      .mockImplementation(async (noteId, patch) => persistedPatch(noteId, patch));
    const controller = new NoteSaveController({ persist });

    controller.queue("note-1", { title: "First note" });
    const flushing = controller.flushAll();
    controller.queue("note-2", { editedContent: "Late blur" });
    finishFirst?.(persistedPatch("note-1", { title: "First note" }));
    await flushing;

    expect(persist).toHaveBeenCalledTimes(2);
    expect(persist).toHaveBeenNthCalledWith(2, "note-2", {
      editedContent: "Late blur",
    });
    expect(controller.hasPending()).toBe(false);
  });

  it("retries a failed save a bounded number of times", async () => {
    vi.useFakeTimers();
    const persist = vi.fn().mockRejectedValue(new Error("database busy"));
    const controller = new NoteSaveController({ persist });

    controller.queue("note-1", { title: "Retry me" });
    await vi.advanceTimersByTimeAsync(NOTE_SAVE_DEBOUNCE_MS * (NOTE_SAVE_MAX_RETRIES + 1));

    expect(persist).toHaveBeenCalledTimes(NOTE_SAVE_MAX_RETRIES + 1);
    expect(controller.hasPending("note-1")).toBe(true);
  });

  it("retries the newest edit queued while an older save is failing", async () => {
    vi.useFakeTimers();
    let rejectFirst: ((error: Error) => void) | undefined;
    const persistedRows = new Map<string, NoteEditablePatch>();
    const persist = vi
      .fn<(noteId: string, patch: NoteEditablePatch) => Promise<NotePatchDto>>()
      .mockImplementationOnce(
        () =>
          new Promise((_resolve, reject) => {
            rejectFirst = reject;
          }),
      )
      .mockImplementation(async (noteId, patch) => {
        persistedRows.set(noteId, patch);
        return persistedPatch(noteId, patch);
      });
    const controller = new NoteSaveController({ persist });

    controller.queue("note-1", { title: "First title", editedContent: "Old body" });
    await vi.advanceTimersByTimeAsync(NOTE_SAVE_DEBOUNCE_MS);
    controller.queue("note-1", { editedContent: "Newest body" });
    await vi.advanceTimersByTimeAsync(NOTE_SAVE_DEBOUNCE_MS);

    rejectFirst?.(new Error("database busy"));
    await vi.runAllTimersAsync();

    expect(persist).toHaveBeenCalledTimes(2);
    expect(persist).toHaveBeenNthCalledWith(2, "note-1", {
      title: "First title",
      editedContent: "Newest body",
    });
    expect(persistedRows.get("note-1")).toEqual({
      title: "First title",
      editedContent: "Newest body",
    });
    expect(controller.hasPending("note-1")).toBe(false);
  });

  it("persists a debounced edit before flush resolves and reads the row back", async () => {
    const rows = new Map<string, NoteEditablePatch>([
      ["note-1", { title: "Draft", editedContent: "Old body" }],
    ]);
    const controller = new NoteSaveController({
      persist: async (noteId, patch) => {
        rows.set(noteId, { ...rows.get(noteId), ...patch });
        return persistedPatch(noteId, rows.get(noteId) ?? {});
      },
    });

    controller.queue("note-1", { title: "Final title", editedContent: "Final body" });
    await controller.flushAll();

    expect(rows.get("note-1")).toEqual({
      title: "Final title",
      editedContent: "Final body",
    });
  });

  it("serializes edits queued while a save is in flight", async () => {
    let finishFirst: ((patch: NotePatchDto) => void) | undefined;
    const persist = vi
      .fn<(noteId: string, patch: NoteEditablePatch) => Promise<NotePatchDto>>()
      .mockImplementationOnce(
        (noteId, patch) =>
          new Promise((resolve) => {
            finishFirst = resolve;
            expect(persistedPatch(noteId, patch).title).toBe("First");
          }),
      )
      .mockImplementation(async (noteId, patch) => persistedPatch(noteId, patch));
    const controller = new NoteSaveController({ persist, debounceMs: 0 });

    controller.queue("note-1", { title: "First" });
    const firstFlush = controller.flush("note-1");
    controller.queue("note-1", { title: "Second" });
    const finalFlush = controller.flush("note-1");
    finishFirst?.(persistedPatch("note-1", { title: "First" }));
    await Promise.all([firstFlush, finalFlush]);

    expect(persist).toHaveBeenNthCalledWith(1, "note-1", { title: "First" });
    expect(persist).toHaveBeenNthCalledWith(2, "note-1", { title: "Second" });
  });
});
