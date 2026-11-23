import type { NoteEditablePatch, NotePatchDto } from "../lib/tauri";

export const NOTE_SAVE_DEBOUNCE_MS = 500;
export const NOTE_SAVE_MAX_FLUSH_PASSES = 8;
export const NOTE_SAVE_MAX_RETRIES = 2;

type NoteSaveControllerOptions = {
  persist: (noteId: string, patch: NoteEditablePatch) => Promise<NotePatchDto>;
  onPersisted?: (patch: NotePatchDto) => void;
  onError?: (error: unknown, noteId: string) => void;
  debounceMs?: number;
};

type NoteSaveResult = { succeeded: true } | { succeeded: false; error: unknown };

/**
 * Coalesces note-row edits per note and serializes writes for the same note.
 *
 * The queue is note-keyed rather than selection-keyed: a blur caused by
 * navigation can safely finish saving the editor that is being torn down
 * without writing its content into the newly selected note.
 */
export class NoteSaveController {
  private readonly pending = new Map<string, NoteEditablePatch>();
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly inFlight = new Map<string, Promise<NoteSaveResult>>();
  private readonly retryAttempts = new Map<string, number>();
  private readonly editVersions = new Map<string, number>();
  private readonly discardVersions = new Map<string, number>();
  private readonly debounceMs: number;

  constructor(private readonly options: NoteSaveControllerOptions) {
    this.debounceMs = options.debounceMs ?? NOTE_SAVE_DEBOUNCE_MS;
  }

  queue(noteId: string, patch: NoteEditablePatch) {
    if (!hasPatchFields(patch)) return;
    this.editVersions.set(noteId, this.getEditVersion(noteId) + 1);
    this.retryAttempts.delete(noteId);
    this.pending.set(noteId, {
      ...this.pending.get(noteId),
      ...patch,
    });
    this.clearTimer(noteId);
    this.timers.set(
      noteId,
      setTimeout(() => {
        this.timers.delete(noteId);
        void this.drain(noteId);
      }, this.debounceMs),
    );
  }

  async saveNow(noteId: string, patch: NoteEditablePatch) {
    this.queue(noteId, patch);
    await this.flush(noteId);
  }

  async flush(noteId: string) {
    this.clearTimer(noteId);
    const result = await this.drain(noteId);
    if (!result.succeeded) throw result.error;
  }

  async flushAll() {
    let lastError: unknown;
    for (let pass = 0; pass < NOTE_SAVE_MAX_FLUSH_PASSES; pass += 1) {
      for (const noteId of [...this.timers.keys()]) {
        this.clearTimer(noteId);
      }
      const noteIds = new Set([...this.pending.keys(), ...this.inFlight.keys()]);
      if (noteIds.size > 0) {
        const results = await Promise.all([...noteIds].map((noteId) => this.drain(noteId)));
        for (const result of results) {
          if (!result.succeeded) lastError = result.error;
        }
      }

      // Let promise continuations that were already queued publish any final
      // editor patch before deciding the queue is stable-empty.
      await Promise.resolve();
      if (!this.hasPending()) return;
    }

    if (lastError !== undefined) throw lastError;
    const drainError = new Error(
      `Could not drain pending note saves after ${NOTE_SAVE_MAX_FLUSH_PASSES} passes`,
    );
    this.options.onError?.(drainError, "all");
    throw drainError;
  }

  hasPending(noteId?: string) {
    if (noteId) {
      return this.pending.has(noteId) || this.inFlight.has(noteId);
    }
    return this.pending.size > 0 || this.inFlight.size > 0;
  }

  discard(noteId: string) {
    this.discardVersions.set(noteId, (this.discardVersions.get(noteId) ?? 0) + 1);
    this.editVersions.set(noteId, this.getEditVersion(noteId) + 1);
    this.clearTimer(noteId);
    this.pending.delete(noteId);
    this.retryAttempts.delete(noteId);
    // The request may still settle, but it no longer belongs to an editable
    // note or the shutdown barrier. Its result cannot publish or retry.
    this.inFlight.delete(noteId);
  }

  /** Local edit ordering for asynchronous native-event hydration. */
  getEditVersion(noteId: string) {
    return this.editVersions.get(noteId) ?? 0;
  }

  private async drain(noteId: string): Promise<NoteSaveResult> {
    const active = this.inFlight.get(noteId);
    if (active) {
      const result = await active;
      if (result.succeeded && this.pending.has(noteId)) {
        return this.drain(noteId);
      }
      return result;
    }

    const patch = this.pending.get(noteId);
    if (!patch) return { succeeded: true };
    this.pending.delete(noteId);

    const operation = this.persist(noteId, patch);
    this.inFlight.set(noteId, operation);
    const result = await operation;
    if (this.inFlight.get(noteId) === operation) {
      this.inFlight.delete(noteId);
    }
    if (result.succeeded && this.pending.has(noteId)) {
      return this.drain(noteId);
    }
    return result;
  }

  private async persist(noteId: string, patch: NoteEditablePatch): Promise<NoteSaveResult> {
    const generation = this.discardVersions.get(noteId) ?? 0;
    const discarded = () => generation !== (this.discardVersions.get(noteId) ?? 0);
    try {
      const persisted = await this.options.persist(noteId, patch);
      if (discarded()) return { succeeded: true };
      this.options.onPersisted?.({
        ...persisted,
        ...this.pending.get(noteId),
      });
      this.retryAttempts.delete(noteId);
      return { succeeded: true };
    } catch (error) {
      if (discarded()) return { succeeded: true };
      // Keep the failed fields available for the next edit/flush. Newer values
      // win when the user changed the same field while this write was running.
      this.pending.set(noteId, {
        ...patch,
        ...this.pending.get(noteId),
      });
      this.options.onError?.(error, noteId);
      this.scheduleRetry(noteId);
      return { succeeded: false, error };
    }
  }

  private clearTimer(noteId: string) {
    const timer = this.timers.get(noteId);
    if (timer !== undefined) {
      clearTimeout(timer);
      this.timers.delete(noteId);
    }
  }

  private scheduleRetry(noteId: string) {
    const attempts = (this.retryAttempts.get(noteId) ?? 0) + 1;
    this.retryAttempts.set(noteId, attempts);
    if (attempts > NOTE_SAVE_MAX_RETRIES || this.timers.has(noteId)) return;

    this.timers.set(
      noteId,
      setTimeout(() => {
        this.timers.delete(noteId);
        void this.drain(noteId);
      }, this.debounceMs),
    );
  }
}

function hasPatchFields(patch: NoteEditablePatch) {
  return (
    patch.title !== undefined || patch.editedContent !== undefined || patch.activeTab !== undefined
  );
}
