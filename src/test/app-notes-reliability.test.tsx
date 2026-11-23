import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../app/App";
import { NOTE_PROCESSING_RECONCILE_INTERVAL_MS } from "../app/use-note-processing-events";
import {
  resetCurrentDataPartitionForTests,
  setCurrentDataPartitionName,
} from "../lib/data-partition";
import { MEETING_START_TRANSCRIPTION_EVENT } from "../lib/events";
import type {
  BootstrapResponse,
  NoteDto,
  RecoverableRecordingDto,
  RecordingSessionDto,
} from "../lib/tauri";

type TauriListener = (event: { payload: unknown }) => unknown;

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, TauriListener>(),
  pendingMeetingStartRequest: undefined as
    | { requestId: string; noteId: string; requestedAtMs: number; expired: boolean }
    | undefined,
  listen: vi.fn((event: string, listener: TauriListener) => {
    mocks.listeners.set(event, listener);
    return Promise.resolve(vi.fn());
  }),
  readPendingMeetingStartRequest: vi.fn(async () => mocks.pendingMeetingStartRequest ?? null),
  acknowledgeMeetingStartRequest: vi.fn(async (requestId: string) => {
    if (mocks.pendingMeetingStartRequest?.requestId !== requestId) return false;
    mocks.pendingMeetingStartRequest = undefined;
    return true;
  }),
  getCurrentWindow: vi.fn(),
  bootstrapApp: vi.fn(),
  createNote: vi.fn(),
  createFolder: vi.fn(),
  deleteFolder: vi.fn(),
  renameFolder: vi.fn(),
  assignNoteToFolder: vi.fn(),
  removeNoteFromFolder: vi.fn(),
  listNotes: vi.fn(),
  listFolders: vi.fn(),
  listAgentSessions: vi.fn(),
  getNote: vi.fn(),
  deleteNote: vi.fn(),
  deleteNotes: vi.fn(),
  downloadNoteAudio: vi.fn(),
  revealPath: vi.fn(),
  updateNote: vi.fn(),
  patchNote: vi.fn(),
  completeNoteSaveFlush: vi.fn(async () => true),
  checkRecordingSourceReadiness: vi.fn(),
  openPrivacySettings: vi.fn(),
  startRecording: vi.fn(),
  startMeetingRecording: vi.fn(),
  pauseRecording: vi.fn(),
  resumeRecording: vi.fn(),
  getRecordingStatus: vi.fn(),
  setRecordingPresenceBounds: vi.fn(),
  finishRecording: vi.fn(),
  retryProcessing: vi.fn(),
  recoverRecording: vi.fn(),
  dictationHelperCommand: vi.fn(),
  dictationSettings: vi.fn(),
  listDictationHistory: vi.fn(),
  listDictionaryEntries: vi.fn(),
  deleteDictationHistoryItem: vi.fn(),
  agentHudShow: vi.fn(),
  agentOpenReady: vi.fn().mockResolvedValue(null),
  agentHudHide: vi.fn(),
  playRecordingSound: vi.fn(),
  preloadRecordingSounds: vi.fn(),
  preloadAgentSounds: vi.fn(),
  toast: Object.assign(vi.fn(), {
    dismiss: vi.fn(),
    error: vi.fn(),
    loading: vi.fn(),
    success: vi.fn(),
    warning: vi.fn(),
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: mocks.getCurrentWindow,
}));

vi.mock("../lib/recording-sounds", () => ({
  playRecordingSound: mocks.playRecordingSound,
  preloadRecordingSounds: mocks.preloadRecordingSounds,
}));

vi.mock("../lib/agent-sounds", () => ({
  preloadAgentSounds: mocks.preloadAgentSounds,
}));

vi.mock("../components/ui/Toaster", () => ({
  toast: mocks.toast,
}));

vi.mock("../lib/tauri", () => ({
  listRecordingMarks: vi.fn().mockResolvedValue([]),
  getRecordingAssistanceStatus: vi
    .fn()
    .mockResolvedValue({ epoch: 0, enabled: false, hasRecentContext: false }),
  appPreferencesGet: vi.fn().mockResolvedValue({ hideFromDock: false }),
  appPreferencesSet: vi.fn(async (request: { hideFromDock: boolean }) => request),
  dictationCapabilities: vi.fn().mockResolvedValue({
    capabilities: {
      available: true,
      platform: "macos",
      shortcuts: true,
      paste: true,
      microphoneSelection: true,
      accessibilityPermission: true,
      systemAudio: true,
    },
  }),
  primeGeneratedVideoDir: vi.fn().mockResolvedValue(undefined),
  computerUseBeginRun: vi.fn().mockResolvedValue(undefined),
  computerUseEndRun: vi.fn().mockResolvedValue(undefined),
  computerUseStop: vi.fn().mockResolvedValue(undefined),
  LIVE_TRANSCRIPT_EVENT: "live-transcript-event",
  RECORDING_TELEMETRY_EVENT: "recording-telemetry",
  NOTE_PROCESSING_PROGRESS_EVENT: "note-processing-progress",
  NOTE_CALENDAR_CONTEXT_UPDATED_EVENT: "clovy://note-calendar-context-updated",
  NOTE_SAVE_FLUSH_REQUESTED_EVENT: "clovy://flush-pending-note-saves",
  bootstrapApp: mocks.bootstrapApp,
  createNote: mocks.createNote,
  createFolder: mocks.createFolder,
  deleteFolder: mocks.deleteFolder,
  renameFolder: mocks.renameFolder,
  assignNoteToFolder: mocks.assignNoteToFolder,
  listSessionFolders: vi.fn(async () => []),
  listCompletedSessions: vi.fn(async () => []),
  setSessionCompleted: vi.fn(async () => undefined),
  listSessionPartitions: vi.fn(async () => []),
  assignSessionToFolder: vi.fn(async () => undefined),
  assignSessionToProfile: vi.fn(async () => undefined),
  removeSessionFromFolder: vi.fn(async () => undefined),
  removeNoteFromFolder: mocks.removeNoteFromFolder,
  listNotes: mocks.listNotes,
  listFolders: mocks.listFolders,
  listAgentSessions: mocks.listAgentSessions,
  getNote: mocks.getNote,
  deleteNote: mocks.deleteNote,
  deleteNotes: mocks.deleteNotes,
  downloadNoteAudio: mocks.downloadNoteAudio,
  revealPath: mocks.revealPath,
  updateNote: mocks.updateNote,
  patchNote: mocks.patchNote,
  completeNoteSaveFlush: mocks.completeNoteSaveFlush,
  checkRecordingSourceReadiness: mocks.checkRecordingSourceReadiness,
  openPrivacySettings: mocks.openPrivacySettings,
  startRecording: mocks.startRecording,
  pauseRecording: mocks.pauseRecording,
  resumeRecording: mocks.resumeRecording,
  getRecordingStatus: mocks.getRecordingStatus,
  setRecordingPresenceBounds: mocks.setRecordingPresenceBounds,
  finishRecording: mocks.finishRecording,
  retryProcessing: mocks.retryProcessing,
  recoverRecording: mocks.recoverRecording,
  dictationHelperCommand: mocks.dictationHelperCommand,
  dictationSettings: mocks.dictationSettings,
  listDictationHistory: mocks.listDictationHistory,
  listDictionaryEntries: mocks.listDictionaryEntries,
  deleteDictationHistoryItem: mocks.deleteDictationHistoryItem,
  agentHudShow: mocks.agentHudShow,
  agentOpenReady: mocks.agentOpenReady,
  agentHudHide: mocks.agentHudHide,
  pendingMeetingStartRequest: mocks.readPendingMeetingStartRequest,
  acknowledgeMeetingStartRequest: mocks.acknowledgeMeetingStartRequest,
  startMeetingRecording: mocks.startMeetingRecording,
  // The agent workspace mounts at launch; a quiet, not-running bridge keeps
  // these tests focused on the meetings surfaces.
  hermesBridgeStatus: vi.fn(async () => ({ running: false })),
  listAgentTasks: vi.fn(async () => ({ items: [] })),
  providerModelSettings: vi.fn(async () => ({
    settings: { generationModel: "" },
  })),
  hermesAgentCliAccess: vi.fn(async () => ({ enabled: false })),
  hermesBrowserAccess: vi.fn(async () => ({ enabled: false })),
  listProviderModels: vi.fn(async () => ({
    mode: "generation",
    modelType: "text",
    selectedModel: "",
    models: [],
  })),
}));

const now = "2026-05-19T10:00:00Z";

function note(overrides: Partial<NoteDto> = {}): NoteDto {
  return {
    id: "note-1",
    title: "First note",
    preview: "Preview",
    processingStatus: "ready",
    folderIds: [],
    createdAt: now,
    updatedAt: now,
    generatedContent: "Existing note",
    activeTab: "notes",
    ...overrides,
  };
}

function recording(overrides: Partial<RecordingSessionDto> = {}) {
  return {
    id: "rec-1",
    noteId: "note-1",
    sourceMode: "microphonePlusSystem" as const,
    state: "recording" as const,
    startedAt: now,
    elapsedMs: 0,
    level: { peak: 0, rms: 0, recentPeaks: [] },
    sources: [],
    warnings: [],
    ...overrides,
  };
}

function recovery(overrides: Partial<RecoverableRecordingDto> = {}): RecoverableRecordingDto {
  return {
    sessionId: "rec-1",
    noteId: "note-1",
    sourceMode: "microphonePlusSystem",
    startedAt: now,
    partialPathPresent: true,
    finalPathPresent: true,
    bytesFound: 2048,
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolver) => {
    resolve = resolver;
  });
  return { promise, resolve };
}

function stubNoteDetailScroller(initialScrollTop: number) {
  const element = document.querySelector<HTMLElement>(".note-detail-scroll");
  if (!element) throw new Error("Expected the note detail scroller to be mounted");
  let scrollTop = initialScrollTop;
  Object.defineProperties(element, {
    scrollHeight: { configurable: true, get: () => 1000 },
    clientHeight: { configurable: true, get: () => 400 },
    scrollTop: {
      configurable: true,
      get: () => scrollTop,
      set: (value: number) => {
        scrollTop = value;
      },
    },
  });
  const scrollTo = vi.fn(({ top }: ScrollToOptions) => {
    scrollTop = Math.min(Number(top), 600);
    element.dispatchEvent(new Event("scroll"));
  });
  Object.defineProperty(element, "scrollTo", { configurable: true, value: scrollTo });
  return {
    element,
    scrollTo,
    setScrollTop(value: number) {
      scrollTop = value;
    },
  };
}

describe("notes recording reliability", () => {
  const first = note();
  // Draft on purpose: drafts were the worst case for the wrong-note
  // optimistic update — a falsely stamped "transcribing" draft locks its
  // record button and shimmers forever.
  const second = note({
    id: "note-2",
    title: "Second note",
    processingStatus: "draft",
    generatedContent: undefined,
  });

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listeners.clear();
    mocks.pendingMeetingStartRequest = undefined;
    mocks.readPendingMeetingStartRequest.mockImplementation(
      async () => mocks.pendingMeetingStartRequest ?? null,
    );
    mocks.acknowledgeMeetingStartRequest.mockImplementation(async (requestId: string) => {
      if (mocks.pendingMeetingStartRequest?.requestId !== requestId) return false;
      mocks.pendingMeetingStartRequest = undefined;
      return true;
    });
    resetCurrentDataPartitionForTests();
    mocks.listFolders.mockResolvedValue([]);
    mocks.listAgentSessions.mockResolvedValue([]);

    const payload: BootstrapResponse = {
      folders: [],
      notes: [first, second],
      activeRecoveries: [],
    };

    mocks.getCurrentWindow.mockReturnValue({
      show: vi.fn().mockResolvedValue(undefined),
      unminimize: vi.fn().mockResolvedValue(undefined),
      setFocus: vi.fn().mockResolvedValue(undefined),
      startDragging: vi.fn().mockResolvedValue(undefined),
    });
    mocks.bootstrapApp.mockResolvedValue(payload);
    // The meeting-detected start path creates a fresh note to record into; this
    // suite asserts recording lands on note-1, so the fresh note IS note-1.
    mocks.createNote.mockResolvedValue(first);
    mocks.deleteNote.mockResolvedValue(undefined);
    mocks.deleteNotes.mockResolvedValue(undefined);
    mocks.downloadNoteAudio.mockResolvedValue({
      path: "/Users/alex/Downloads/First note audio.wav",
      fileName: "First note audio.wav",
      sourceCount: 1,
    });
    mocks.revealPath.mockResolvedValue(undefined);
    mocks.listNotes.mockResolvedValue({ items: [first, second] });
    mocks.getNote.mockImplementation(async (noteId: string) =>
      noteId === "note-2" ? second : first,
    );
    mocks.checkRecordingSourceReadiness.mockResolvedValue({
      sourceMode: "microphonePlusSystem",
      sources: [
        { source: "microphone", ready: true },
        { source: "system", ready: true, permissionState: "granted" },
      ],
    });
    mocks.startRecording.mockResolvedValue(recording());
    mocks.startMeetingRecording.mockResolvedValue({
      status: "started",
      note: first,
      recording: recording(),
    });
    mocks.getRecordingStatus.mockResolvedValue({
      sessionId: "rec-1",
      state: "recording",
      elapsedMs: 500,
      level: { peak: 0.2, rms: 0.1, recentPeaks: [0.2] },
      silenceWarning: false,
      bytesWritten: 2048,
    });
    mocks.dictationHelperCommand.mockResolvedValue(undefined);
    mocks.dictationSettings.mockResolvedValue({
      settings: {
        pushToTalkShortcut: {
          code: "KeyD",
          label: "Ctrl+Opt+D",
          pressCount: 1,
          modifiers: {
            command: false,
            control: true,
            option: true,
            shift: false,
            function: false,
          },
        },
        toggleShortcut: {
          code: "KeyT",
          label: "Ctrl+Opt+T",
          pressCount: 1,
          modifiers: {
            command: false,
            control: true,
            option: true,
            shift: false,
            function: false,
          },
        },
        microphone: {},
        style: "standard",
      },
    });
    mocks.listDictationHistory.mockResolvedValue({
      items: [],
      retentionDays: 7,
    });
    mocks.listDictionaryEntries.mockResolvedValue([]);
    mocks.deleteDictationHistoryItem.mockResolvedValue(undefined);
    mocks.updateNote.mockImplementation(async (input) => ({
      ...first,
      ...input,
    }));
    mocks.patchNote.mockImplementation(async (noteId, patch) => ({
      id: noteId,
      title: patch.title ?? first.title,
      preview: first.preview,
      editedContent: patch.editedContent ?? first.editedContent,
      activeTab: patch.activeTab ?? first.activeTab,
      updatedAt: now,
    }));
  });

  async function startRecordingOnFirstNote() {
    render(<App />);
    await waitFor(() => expect(mocks.listeners.has(MEETING_START_TRANSCRIPTION_EVENT)).toBe(true));
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    mocks.pendingMeetingStartRequest = {
      requestId: "meeting-request-1",
      noteId: "note-1",
      requestedAtMs: Date.now(),
      expired: false,
    };
    await act(async () => {
      await mocks.listeners.get(MEETING_START_TRANSCRIPTION_EVENT)?.({
        payload: undefined,
      });
    });
    await waitFor(() =>
      expect(mocks.startMeetingRecording).toHaveBeenCalledWith(
        "meeting-request-1",
        "microphonePlusSystem",
      ),
    );
  }

  async function startRecordingDirectlyOnFirstNote() {
    render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(screen.getByRole("button", { name: "First note Preview" }));
    await userEvent.click(await screen.findByRole("button", { name: "Record" }));
    await waitFor(() =>
      expect(mocks.startRecording).toHaveBeenCalledWith("note-1", "microphonePlusSystem"),
    );
  }

  it("keeps the last clicked note open when an earlier open resolves later", async () => {
    render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    let resolveSecond: ((value: NoteDto) => void) | undefined;
    mocks.getNote.mockImplementation((noteId: string) =>
      noteId === "note-2"
        ? new Promise<NoteDto>((resolve) => {
            resolveSecond = resolve;
          })
        : Promise.resolve(first),
    );

    await userEvent.click(screen.getByRole("button", { name: /^Second note/ }));
    await userEvent.click(screen.getByRole("button", { name: /^First note/ }));
    await act(async () => {
      resolveSecond?.(second);
    });

    expect(screen.getAllByText("First note").length).toBeGreaterThan(0);
    expect(screen.queryByText(/Second note/)).toBeNull();
  });

  it("swaps notes when the data partition switches", async () => {
    render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));

    const workNote = note({ id: "note-work", title: "Work data partition note" });
    mocks.listNotes.mockResolvedValue({ items: [workNote] });
    mocks.getNote.mockResolvedValue(workNote);
    const listCallsBefore = mocks.listNotes.mock.calls.length;

    await act(async () => {
      setCurrentDataPartitionName("work");
    });

    await waitFor(() => expect(mocks.listNotes.mock.calls.length).toBeGreaterThan(listCallsBefore));
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    expect(await screen.findByText("Work data partition note")).toBeInTheDocument();
    expect(screen.queryByText("First note")).toBeNull();
  });

  it("shows calendar context as soon as the backend matches the open note", async () => {
    await startRecordingOnFirstNote();
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(await screen.findByRole("button", { name: /First note Preview/ }));
    expect(await screen.findByDisplayValue("First note")).toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.listeners.has("clovy://note-calendar-context-updated")).toBe(true),
    );

    await act(async () => {
      await mocks.listeners.get("clovy://note-calendar-context-updated")?.({
        payload: {
          ...first,
          title: "Product review",
          calendarEvent: {
            eventId: "event-1",
            title: "Product review",
            startAt: "2026-07-20T14:00:00Z",
            endAt: "2026-07-20T14:30:00Z",
            accountEmail: "june@example.com",
          },
        },
      });
    });

    expect(
      await screen.findByLabelText("Matched to Product review in Google Calendar"),
    ).toBeInTheDocument();
  });

  it("flushes pending note edits and acknowledges the native app-quit barrier", async () => {
    render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(screen.getByRole("button", { name: /First note Preview/ }));
    const title = await screen.findByDisplayValue("First note");
    mocks.patchNote.mockClear();

    await userEvent.type(title, " unsaved");
    await act(async () => {
      await mocks.listeners.get("clovy://flush-pending-note-saves")?.({
        payload: { requestId: "flush-1" },
      });
    });

    expect(mocks.patchNote).toHaveBeenCalledWith("note-1", {
      title: "First note unsaved",
    });
    expect(mocks.completeNoteSaveFlush).toHaveBeenCalledWith("flush-1");
  });

  it("does not acknowledge app quit when pending note persistence fails", async () => {
    mocks.patchNote.mockRejectedValue(new Error("database busy"));
    render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(screen.getByRole("button", { name: /First note Preview/ }));
    const title = await screen.findByDisplayValue("First note");
    mocks.completeNoteSaveFlush.mockClear();

    await userEvent.type(title, " unsaved");
    await act(async () => {
      await mocks.listeners.get("clovy://flush-pending-note-saves")?.({
        payload: { requestId: "flush-failed" },
      });
    });

    expect(mocks.completeNoteSaveFlush).not.toHaveBeenCalled();
    expect(await screen.findByText("database busy")).toBeInTheDocument();
  });

  it("ignores calendar context without data partition provenance after a renderer reload", async () => {
    render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await waitFor(() =>
      expect(mocks.listeners.has("clovy://note-calendar-context-updated")).toBe(true),
    );

    await act(async () => {
      await mocks.listeners.get("clovy://note-calendar-context-updated")?.({
        payload: {
          ...first,
          title: "Stale calendar note",
          calendarEvent: {
            eventId: "event-stale",
            title: "Stale calendar note",
            startAt: "2026-07-20T14:00:00Z",
            endAt: "2026-07-20T14:30:00Z",
            accountEmail: "unknown-partition@example.com",
          },
        },
      });
    });

    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    expect(screen.queryByText("Stale calendar note")).toBeNull();
    expect(screen.queryByText("unknown-partition@example.com")).toBeNull();
    expect(await screen.findByText("First note")).toBeInTheDocument();
  });

  it("retires an old-partition recording note as soon as the recording stops", async () => {
    const workNote = note({ id: "note-work", title: "Work data partition note" });
    mocks.finishRecording.mockResolvedValue({
      note: { ...first, processingStatus: "transcribing" },
      recording: recording({ state: "ready" }),
      validation: {},
      processingStarted: true,
    });

    await startRecordingOnFirstNote();

    mocks.listNotes.mockResolvedValue({ items: [workNote] });
    mocks.getNote.mockImplementation(async (noteId: string) =>
      noteId === workNote.id ? workNote : first,
    );
    const listCallsBeforeSwitch = mocks.listNotes.mock.calls.length;
    await act(async () => {
      setCurrentDataPartitionName("work");
    });

    await waitFor(() =>
      expect(mocks.listNotes.mock.calls.length).toBeGreaterThan(listCallsBeforeSwitch),
    );
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith(first.id));
    const listCallsBeforeFinish = mocks.listNotes.mock.calls.length;
    await userEvent.click(await screen.findByRole("button", { name: "Done" }));
    await waitFor(() => expect(mocks.finishRecording).toHaveBeenCalledWith("rec-1"));
    await waitFor(() =>
      expect(mocks.listNotes.mock.calls.length).toBeGreaterThan(listCallsBeforeFinish),
    );

    const notesTab = await screen.findByRole("tab", { name: "Notes" });
    expect(notesTab).toHaveAttribute("data-active", "true");
    await userEvent.click(screen.getByRole("button", { name: "Meeting notes" }));
    expect(await screen.findByText("Work data partition note")).toBeInTheDocument();
    expect(screen.queryByText("First note")).toBeNull();

    await act(async () => {
      await mocks.listeners.get("clovy://note-calendar-context-updated")?.({
        payload: {
          ...first,
          title: "Old data partition calendar note",
          calendarEvent: {
            eventId: "event-old-partition",
            title: "Old data partition calendar note",
            startAt: "2026-07-20T14:00:00Z",
            endAt: "2026-07-20T14:30:00Z",
            accountEmail: "personal@example.com",
          },
        },
      });
    });

    expect(screen.queryByText("Old data partition calendar note")).toBeNull();
    expect(screen.queryByText("First note")).toBeNull();
  });

  it("hides audio download when the selected note has no finalized audio", async () => {
    render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(screen.getByRole("button", { name: /First note Preview/ }));

    await userEvent.click(screen.getByRole("button", { name: "Note actions" }));

    expect(screen.queryByRole("menuitem", { name: "Download audio" })).not.toBeInTheDocument();
  });

  it.each([
    { format: "wav", sizeBytes: 0, label: "an empty WAV" },
    { format: "mp3", sizeBytes: 2048, label: "a non-WAV artifact" },
  ])("hides audio download for $label", async ({ format, sizeBytes }) => {
    const noteWithoutDownloadableAudio = note({
      audioSources: [
        {
          id: "audio-1",
          source: "microphone",
          format,
          durationMs: 1000,
          sizeBytes,
          checksum: "abc",
          createdAt: now,
        },
      ],
    });
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [noteWithoutDownloadableAudio, second],
      activeRecoveries: [],
    });
    mocks.getNote.mockImplementation(async (noteId: string) =>
      noteId === "note-2" ? second : noteWithoutDownloadableAudio,
    );

    render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(screen.getByRole("button", { name: /First note Preview/ }));
    await userEvent.click(screen.getByRole("button", { name: "Note actions" }));

    expect(screen.queryByRole("menuitem", { name: "Download audio" })).not.toBeInTheDocument();
  });

  it("downloads selected note audio and reveals the saved file from the success toast", async () => {
    const noteWithAudio = note({
      audioSources: [
        {
          id: "audio-1",
          source: "microphone",
          format: "wav",
          durationMs: 1000,
          sizeBytes: 2048,
          checksum: "abc",
          createdAt: now,
        },
      ],
    });
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [noteWithAudio, second],
      activeRecoveries: [],
    });
    mocks.getNote.mockImplementation(async (noteId: string) =>
      noteId === "note-2" ? second : noteWithAudio,
    );

    render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(screen.getByRole("button", { name: /First note Preview/ }));
    await userEvent.click(screen.getByRole("button", { name: "Note actions" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "Download audio" }));

    await waitFor(() => expect(mocks.downloadNoteAudio).toHaveBeenCalledWith("note-1"));
    expect(mocks.toast.success).toHaveBeenCalledWith("Audio downloaded", {
      action: {
        label: "Show file",
        onClick: expect.any(Function),
      },
    });

    const toastOptions = mocks.toast.success.mock.calls[0]?.[1] as {
      action: { onClick: () => void };
    };
    toastOptions.action.onClick();

    await waitFor(() =>
      expect(mocks.revealPath).toHaveBeenCalledWith("/Users/alex/Downloads/First note audio.wav"),
    );
  });

  it("shows download and reveal failures as error toasts", async () => {
    const noteWithAudio = note({
      audio: {
        id: "audio-1",
        source: "microphone",
        format: "wav",
        durationMs: 1000,
        sizeBytes: 2048,
        checksum: "abc",
        createdAt: now,
      },
    });
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [noteWithAudio, second],
      activeRecoveries: [],
    });
    mocks.getNote.mockImplementation(async (noteId: string) =>
      noteId === "note-2" ? second : noteWithAudio,
    );
    mocks.downloadNoteAudio.mockRejectedValueOnce(new Error("Audio download failed"));

    render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(screen.getByRole("button", { name: /First note Preview/ }));
    await userEvent.click(screen.getByRole("button", { name: "Note actions" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "Download audio" }));

    await waitFor(() => expect(mocks.toast.error).toHaveBeenCalledWith("Audio download failed"));

    mocks.toast.error.mockClear();
    mocks.downloadNoteAudio.mockResolvedValueOnce({
      path: "/Users/alex/Downloads/First note audio.wav",
      fileName: "First note audio.wav",
      sourceCount: 1,
    });
    mocks.revealPath.mockRejectedValueOnce(new Error("Could not show file"));
    await userEvent.click(screen.getByRole("button", { name: "Note actions" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "Download audio" }));
    await waitFor(() => expect(mocks.toast.success).toHaveBeenCalled());

    const latestToastOptions = mocks.toast.success.mock.calls.at(-1)?.[1] as {
      action: { onClick: () => void };
    };
    latestToastOptions.action.onClick();

    await waitFor(() => expect(mocks.toast.error).toHaveBeenCalledWith("Could not show file"));
  });

  it("stays on notes after deleting the last note", async () => {
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [first],
      activeRecoveries: [],
    });
    mocks.listNotes.mockResolvedValue({ items: [] });

    render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));

    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(screen.getByRole("button", { name: "Actions for First note" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "Delete note" }));
    await userEvent.click(screen.getByRole("button", { name: "Delete note" }));

    await waitFor(() => expect(mocks.deleteNote).toHaveBeenCalledWith("note-1"));
    expect(
      screen.getByRole("button", { name: "Meeting notes", current: "page" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Capture your first meeting" })).toBeInTheDocument();
  });

  it("stays on notes after bulk deleting every note", async () => {
    mocks.listNotes.mockResolvedValue({ items: [] });

    render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));

    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(screen.getByRole("checkbox", { name: "Select First note" }));
    await userEvent.click(screen.getByRole("checkbox", { name: "Select Second note" }));
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    await userEvent.click(screen.getByRole("button", { name: "Delete notes" }));

    await waitFor(() => expect(mocks.deleteNotes).toHaveBeenCalledWith(["note-1", "note-2"]));
    expect(
      screen.getByRole("button", { name: "Meeting notes", current: "page" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Capture your first meeting" })).toBeInTheDocument();
  });

  it("does not mark a different note transcribing when finishing a recording started elsewhere", async () => {
    mocks.finishRecording.mockResolvedValue({
      note: { ...first, processingStatus: "transcribing" },
      recording: recording({ state: "ready" }),
      validation: {},
      processingStarted: true,
    });

    await startRecordingOnFirstNote();

    // Browse to another note while the recording keeps running on note-1.
    await userEvent.click(screen.getByRole("button", { name: "Meeting notes", current: "page" }));
    await userEvent.click(screen.getByRole("button", { name: /Second note Preview/ }));
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-2"));

    // Off the recording's note, the sidebar indicator stands in for the
    // in-note bar. Click it back to note-1, where Done lives.
    const indicator = await screen.findByRole("button", {
      name: "Open recording: First note",
    });
    await userEvent.click(indicator);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await userEvent.click(await screen.findByRole("button", { name: "Done" }));
    await waitFor(() => expect(mocks.finishRecording).toHaveBeenCalledWith("rec-1"));

    // note-2 must not pick up note-1's optimistic "transcribing" lock.
    mocks.getNote.mockClear();
    await userEvent.click(screen.getByRole("button", { name: "Meeting notes", current: "page" }));
    await userEvent.click(screen.getByRole("button", { name: /Second note Preview/ }));
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-2"));
    expect(screen.queryByText(/Transcribing audio/)).not.toBeInTheDocument();
  });

  it("keeps provisional transcript visible after Stop while saved-audio processing is pending", async () => {
    const pendingFinish = deferred<never>();
    mocks.finishRecording.mockReturnValue(pendingFinish.promise);

    await startRecordingOnFirstNote();
    await waitFor(() => expect(mocks.listeners.has("live-transcript-event")).toBe(true));
    await act(async () => {
      await mocks.listeners.get("live-transcript-event")?.({
        payload: {
          noteId: "note-1",
          sessionId: "rec-1",
          sourceMode: "microphonePlusSystem",
          source: "microphone",
          segmentId: "microphone-0",
          startMs: 0,
          endMs: 4000,
          text: "Provisional words survive Stop",
          stability: "final",
        },
      });
    });

    await userEvent.click(screen.getByRole("button", { name: "Transcription" }));
    expect(await screen.findByText("Provisional words survive Stop")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(mocks.finishRecording).toHaveBeenCalledWith("rec-1"));
    expect(screen.getByText("Provisional words survive Stop")).toBeInTheDocument();
  });

  it("removes a discarded echo preview when the terminal event arrives", async () => {
    const complete = {
      ...first,
      activeTab: "transcription" as const,
      processingStatus: "ready" as const,
      updatedAt: "2026-05-19T10:00:05Z",
      sourceTranscripts: [
        {
          id: "system-final",
          recordingSessionId: "rec-1",
          source: "system" as const,
          startMs: 0,
          endMs: 4000,
          text: "System final",
          status: "succeeded" as const,
        },
      ],
    };
    mocks.finishRecording.mockResolvedValue({
      note: { ...first, processingStatus: "transcribing" },
      recording: recording({ state: "ready" }),
      validation: {},
      processingStarted: true,
    });
    await startRecordingDirectlyOnFirstNote();
    await waitFor(() => expect(mocks.listeners.has("live-transcript-event")).toBe(true));
    await act(async () => {
      await mocks.listeners.get("live-transcript-event")?.({
        payload: {
          noteId: "note-1",
          sessionId: "rec-1",
          sourceMode: "microphonePlusSystem",
          source: "microphone",
          segmentId: "mic-echo",
          startMs: 0,
          endMs: 4000,
          text: "Echo preview",
          stability: "final",
        },
      });
    });
    await userEvent.click(screen.getByRole("button", { name: "Transcription" }));
    expect(await screen.findByText("Echo preview")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(mocks.finishRecording).toHaveBeenCalled());
    mocks.getNote.mockResolvedValue(complete);
    await act(async () => {
      await mocks.listeners.get("note-processing-progress")?.({
        payload: {
          noteId: "note-1",
          recordingSessionId: "rec-1",
          stage: "done",
          processingStatus: "ready",
          revision: complete.updatedAt,
          discardedSpans: [{ sessionId: "rec-1", source: "microphone", startMs: 0, endMs: 4000 }],
        },
      });
    });
    await waitFor(() => expect(screen.queryByText("Echo preview")).not.toBeInTheDocument());
    expect(await screen.findByText("System final")).toBeInTheDocument();
  });

  it("retains session A discard when session B supersedes its pending hydration", async () => {
    mocks.finishRecording.mockResolvedValue({
      note: { ...first, activeTab: "transcription", processingStatus: "transcribing" },
      recording: recording({ state: "ready" }),
      validation: {},
      processingStarted: true,
    });
    await startRecordingDirectlyOnFirstNote();
    await waitFor(() => expect(mocks.listeners.has("live-transcript-event")).toBe(true));
    await act(async () => {
      await mocks.listeners.get("live-transcript-event")?.({
        payload: {
          noteId: "note-1",
          sessionId: "rec-1",
          sourceMode: "microphonePlusSystem",
          source: "microphone",
          segmentId: "echo-a",
          startMs: 0,
          endMs: 8000,
          text: "Queued echo preview",
          stability: "final",
        },
      });
    });
    await userEvent.click(screen.getByRole("button", { name: "Transcription" }));
    expect(await screen.findByText("Queued echo preview")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(mocks.finishRecording).toHaveBeenCalled());
    const pendingA = deferred<NoteDto>();
    const finalNote = {
      ...first,
      activeTab: "transcription" as const,
      processingStatus: "ready" as const,
      updatedAt: "2026-05-19T10:00:06Z",
      sourceTranscripts: [
        {
          id: "final-a",
          recordingSessionId: "rec-1",
          source: "microphone" as const,
          startMs: 4000,
          endMs: 8000,
          text: "Final words",
          status: "succeeded" as const,
        },
      ],
    };
    mocks.getNote
      .mockReset()
      .mockImplementationOnce(() => pendingA.promise)
      .mockResolvedValue(finalNote);
    const emit = async (payload: object) => {
      await act(async () => {
        await mocks.listeners.get("note-processing-progress")?.({ payload });
      });
    };
    await emit({
      noteId: "note-1",
      recordingSessionId: "rec-1",
      stage: "done",
      processingStatus: "ready",
      revision: "2026-05-19T10:00:05Z",
      discardedSpans: [{ sessionId: "rec-1", source: "microphone", startMs: 0, endMs: 4000 }],
    });
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(1));
    expect(screen.getByText("Queued echo preview")).toBeInTheDocument();
    await emit({
      noteId: "note-1",
      recordingSessionId: "rec-2",
      stage: "transcribing",
      processingStatus: "transcribing",
      revision: "2026-05-19T10:00:05Z",
    });
    await act(async () => {
      pendingA.resolve(finalNote);
      await pendingA.promise;
    });
    expect(screen.getByText("Queued echo preview")).toBeInTheDocument();
    await emit({
      noteId: "note-1",
      recordingSessionId: "rec-2",
      stage: "done",
      processingStatus: "ready",
      revision: finalNote.updatedAt,
      discardedSpans: [],
    });
    expect(await screen.findByText("Final words")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("Queued echo preview")).not.toBeInTheDocument());
  });

  it("follows live transcription until the reader scrolls upward, then resumes at the bottom", async () => {
    await startRecordingOnFirstNote();
    await waitFor(() => expect(mocks.listeners.has("live-transcript-event")).toBe(true));
    const scroller = stubNoteDetailScroller(100);

    await userEvent.click(screen.getByRole("button", { name: "Transcription" }));
    await waitFor(() =>
      expect(scroller.scrollTo).toHaveBeenCalledWith({ top: 1000, behavior: "smooth" }),
    );
    scroller.scrollTo.mockClear();

    const emitPreview = async (segmentId: string, text: string, startMs: number) => {
      await act(async () => {
        await mocks.listeners.get("live-transcript-event")?.({
          payload: {
            noteId: "note-1",
            sessionId: "rec-1",
            sourceMode: "microphonePlusSystem",
            source: "microphone",
            segmentId,
            startMs,
            endMs: startMs + 4000,
            text,
            stability: "final",
          },
        });
      });
    };

    await emitPreview("microphone-0", "First live words", 0);
    await waitFor(() => expect(scroller.scrollTo).toHaveBeenCalledTimes(1));

    scroller.scrollTo.mockClear();
    scroller.setScrollTop(100);
    fireEvent.wheel(scroller.element);
    fireEvent.scroll(scroller.element);
    await emitPreview("microphone-1", "Words while reading above", 4000);
    expect(scroller.scrollTo).not.toHaveBeenCalled();

    scroller.setScrollTop(600);
    fireEvent.scroll(scroller.element);
    await emitPreview("microphone-2", "Following resumes", 8000);
    await waitFor(() => expect(scroller.scrollTo).toHaveBeenCalledTimes(1));
  });

  it("does not clear a newer preview when the note still has queued recordings", async () => {
    mocks.finishRecording.mockResolvedValue({
      note: { ...first, activeTab: "transcription", queuedRecordings: 1 },
      recording: recording({ state: "ready" }),
      validation: {},
      processingStarted: true,
    });

    await startRecordingOnFirstNote();
    await waitFor(() => expect(mocks.listeners.has("live-transcript-event")).toBe(true));
    await act(async () => {
      await mocks.listeners.get("live-transcript-event")?.({
        payload: {
          noteId: "note-1",
          sessionId: "rec-1",
          sourceMode: "microphonePlusSystem",
          source: "microphone",
          segmentId: "microphone-queued",
          startMs: 0,
          endMs: 4000,
          text: "Queued session preview remains",
          stability: "final",
        },
      });
    });

    await userEvent.click(screen.getByRole("button", { name: "Transcription" }));
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(mocks.finishRecording).toHaveBeenCalledWith("rec-1"));
    expect(await screen.findByText("Queued session preview remains")).toBeInTheDocument();
  });

  it("shows a sidebar recorder indicator off the recording's note and reopens it on click", async () => {
    await startRecordingOnFirstNote();

    // Browse away from the recording note while the take keeps running.
    await userEvent.click(screen.getByRole("button", { name: "Meeting notes", current: "page" }));
    await userEvent.click(screen.getByRole("button", { name: /Second note Preview/ }));
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-2"));

    // The sidebar indicator stands in for the in-note bar, titled after note-1.
    const indicator = await screen.findByRole("button", {
      name: "Open recording: First note",
    });

    mocks.getNote.mockClear();
    await userEvent.click(indicator);

    // Clicking it jumps back to the recording's note, where it yields to the
    // in-note recorder bar.
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "Open recording: First note" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("updates recording status from native telemetry without polling", async () => {
    await startRecordingOnFirstNote();
    await screen.findByRole("button", { name: "Done" });
    await waitFor(() => expect(mocks.listeners.has("recording-telemetry")).toBe(true));
    expect(mocks.getRecordingStatus).not.toHaveBeenCalled();

    await act(async () => {
      await mocks.listeners.get("recording-telemetry")?.({
        payload: {
          sessionId: "rec-1",
          state: "recording",
          elapsedMs: 1500,
          level: { peak: 0.2, rms: 0.1, recentPeaks: [0.2] },
          silenceWarning: false,
          sources: [],
          warnings: [],
        },
      });
    });
    expect(await screen.findByText("00:01")).toBeInTheDocument();
    expect(mocks.getRecordingStatus).not.toHaveBeenCalled();

    await act(async () => {
      await mocks.listeners.get("recording-telemetry")?.({
        payload: {
          sessionId: "rec-1",
          state: "idle",
          elapsedMs: 1500,
          level: { peak: 0, rms: 0, recentPeaks: [] },
          silenceWarning: false,
          sources: [],
          warnings: [],
        },
      });
    });
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Done" })).not.toBeInTheDocument(),
    );
  });

  it("ignores meeting-start signals while a recording is already live", async () => {
    await startRecordingOnFirstNote();

    mocks.createNote.mockClear();
    mocks.startMeetingRecording.mockClear();

    mocks.pendingMeetingStartRequest = {
      requestId: "meeting-request-2",
      noteId: "meeting-note-2",
      requestedAtMs: Date.now(),
      expired: false,
    };
    await act(async () => {
      await mocks.listeners.get(MEETING_START_TRANSCRIPTION_EVENT)?.({
        payload: undefined,
      });
    });

    expect(mocks.createNote).not.toHaveBeenCalled();
    expect(mocks.startMeetingRecording).not.toHaveBeenCalled();
    expect(mocks.acknowledgeMeetingStartRequest).toHaveBeenCalledWith("meeting-request-2");
  });

  it("serializes duplicate meeting events while native startup is pending", async () => {
    const fresh = note({
      id: "fresh-note",
      title: "New note",
      generatedContent: undefined,
      processingStatus: "draft",
    });
    const pendingStart = deferred<{
      status: "started";
      note: NoteDto;
      recording: RecordingSessionDto;
    }>();
    mocks.startMeetingRecording.mockReturnValue(pendingStart.promise);

    render(<App />);
    await waitFor(() => expect(mocks.listeners.has(MEETING_START_TRANSCRIPTION_EVENT)).toBe(true));
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));

    mocks.pendingMeetingStartRequest = {
      requestId: "meeting-request-1",
      noteId: fresh.id,
      requestedAtMs: Date.now(),
      expired: false,
    };
    act(() => {
      void mocks.listeners.get(MEETING_START_TRANSCRIPTION_EVENT)?.({
        payload: undefined,
      });
    });
    await waitFor(() => expect(mocks.startMeetingRecording).toHaveBeenCalledTimes(1));

    act(() => {
      void mocks.listeners.get(MEETING_START_TRANSCRIPTION_EVENT)?.({
        payload: undefined,
      });
    });

    expect(mocks.startMeetingRecording).toHaveBeenCalledTimes(1);

    await act(async () => {
      pendingStart.resolve({
        status: "started",
        note: fresh,
        recording: recording({ noteId: fresh.id }),
      });
      await pendingStart.promise;
    });

    await waitFor(() =>
      expect(mocks.acknowledgeMeetingStartRequest).toHaveBeenCalledWith("meeting-request-1"),
    );
    expect(mocks.startMeetingRecording).toHaveBeenCalledTimes(1);
  });

  it("acknowledges a visible native startup failure", async () => {
    mocks.startMeetingRecording.mockResolvedValue({
      status: "failed",
      error: {
        code: "source_not_ready",
        message: "Microphone is not ready.",
      },
    });

    render(<App />);
    await waitFor(() => expect(mocks.listeners.has(MEETING_START_TRANSCRIPTION_EVENT)).toBe(true));
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));

    mocks.pendingMeetingStartRequest = {
      requestId: "meeting-request-1",
      noteId: "failed-meeting-note",
      requestedAtMs: Date.now(),
      expired: false,
    };
    await act(async () => {
      await mocks.listeners.get(MEETING_START_TRANSCRIPTION_EVENT)?.({
        payload: undefined,
      });
    });
    expect(await screen.findByText("Microphone is not ready.")).toBeInTheDocument();
    expect(mocks.acknowledgeMeetingStartRequest).toHaveBeenCalledWith("meeting-request-1");
  });

  it("releases the start latch after a terminal native failure", async () => {
    const fresh = note({ id: "second-fresh-note", title: "Second fresh note" });
    mocks.startMeetingRecording
      .mockResolvedValueOnce({
        status: "failed",
        error: {
          code: "capture_start_timeout",
          message: "Could not start the microphone. Try again.",
        },
      })
      .mockResolvedValueOnce({
        status: "started",
        note: fresh,
        recording: recording({ noteId: fresh.id }),
      });

    render(<App />);
    await waitFor(() => expect(mocks.listeners.has(MEETING_START_TRANSCRIPTION_EVENT)).toBe(true));
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));

    mocks.pendingMeetingStartRequest = {
      requestId: "meeting-request-1",
      noteId: "failed-meeting-note",
      requestedAtMs: Date.now(),
      expired: false,
    };
    await act(async () => {
      await mocks.listeners.get(MEETING_START_TRANSCRIPTION_EVENT)?.({
        payload: undefined,
      });
    });
    await waitFor(() => expect(mocks.startMeetingRecording).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mocks.pendingMeetingStartRequest).toBeUndefined());

    mocks.pendingMeetingStartRequest = {
      requestId: "meeting-request-2",
      noteId: fresh.id,
      requestedAtMs: Date.now(),
      expired: false,
    };
    await act(async () => {
      await mocks.listeners.get(MEETING_START_TRANSCRIPTION_EVENT)?.({
        payload: undefined,
      });
    });
    await waitFor(() => expect(mocks.startMeetingRecording).toHaveBeenCalledTimes(2));
  });

  it("retains and retries a request after an IPC failure", async () => {
    mocks.startMeetingRecording
      .mockRejectedValueOnce(new Error("webview IPC was interrupted"))
      .mockResolvedValueOnce({
        status: "failed",
        error: {
          code: "source_not_ready",
          message: "Microphone is not ready.",
        },
      });

    render(<App />);
    await waitFor(() => expect(mocks.listeners.has(MEETING_START_TRANSCRIPTION_EVENT)).toBe(true));
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));

    mocks.pendingMeetingStartRequest = {
      requestId: "meeting-request-1",
      noteId: "failed-meeting-note",
      requestedAtMs: Date.now(),
      expired: false,
    };
    await act(async () => {
      await mocks.listeners.get(MEETING_START_TRANSCRIPTION_EVENT)?.({
        payload: undefined,
      });
    });
    expect(mocks.acknowledgeMeetingStartRequest).not.toHaveBeenCalled();
    await waitFor(() => expect(mocks.startMeetingRecording).toHaveBeenCalledTimes(2), {
      timeout: 2_000,
    });
    await waitFor(() =>
      expect(mocks.acknowledgeMeetingStartRequest).toHaveBeenCalledWith("meeting-request-1"),
    );
  });

  it("clears the recorder presence and disables retry when a recovery is discarded", async () => {
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [first, second],
      activeRecoveries: [recovery()],
    });
    mocks.recoverRecording.mockResolvedValue({
      ...first,
      processingStatus: "failed",
      lastError: "Recording discarded",
      audio: undefined,
      audioSources: [],
    });

    await startRecordingOnFirstNote();

    await userEvent.click(screen.getByRole("button", { name: "Meeting notes", current: "page" }));
    await userEvent.click(screen.getByRole("button", { name: /Second note Preview/ }));
    const indicator = await screen.findByRole("button", {
      name: "Open recording: First note",
    });
    await userEvent.click(indicator);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));

    await userEvent.click(screen.getByRole("button", { name: "Discard" }));

    await waitFor(() => expect(mocks.recoverRecording).toHaveBeenCalledWith("rec-1", "discard"));
    expect(screen.getByText("Recording discarded")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Retry/ })).toBeDisabled();

    await userEvent.click(screen.getByRole("button", { name: "Meeting notes", current: "page" }));
    await userEvent.click(screen.getByRole("button", { name: /Second note Preview/ }));
    expect(
      screen.queryByRole("button", { name: "Open recording: First note" }),
    ).not.toBeInTheDocument();
  });

  it("keeps recovered transcription failures scoped to the failed note", async () => {
    const failedNote = {
      ...first,
      processingStatus: "failed" as const,
      lastError: "Microphone: upstream_provider_failed",
      audioSources: [
        {
          id: "audio-1",
          source: "microphone" as const,
          format: "wav" as const,
          durationMs: 1000,
          sizeBytes: 2048,
          checksum: "abc",
          createdAt: now,
        },
      ],
    };
    let recoveryFailed = false;
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [first, second],
      activeRecoveries: [recovery()],
    });
    mocks.getNote.mockImplementation(async (noteId: string) => {
      if (noteId === "note-2") return second;
      return recoveryFailed ? failedNote : first;
    });
    mocks.recoverRecording.mockImplementation(async () => {
      recoveryFailed = true;
      throw {
        code: "transcription_failed",
        message: "Microphone: The transcription provider could not process this audio.",
      };
    });

    const { container } = render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(screen.getByRole("button", { name: /First note Preview/ }));

    await userEvent.click(screen.getByRole("button", { name: "Recover" }));

    await waitFor(() => expect(mocks.recoverRecording).toHaveBeenCalledWith("rec-1", "validate"));
    await waitFor(() =>
      expect(
        screen.getByText(/Microphone: The transcription provider could not process this audio\./),
      ).toBeInTheDocument(),
    );
    expect(container.querySelector(".note-failure-banner")).not.toBeNull();
    expect(container.querySelector(".error-banner")).toBeNull();
    expect(screen.queryByLabelText("Recoverable recording")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Dictation" }));
    expect(container.querySelector(".note-failure-banner")).toBeNull();
    expect(container.querySelector(".error-banner")).toBeNull();
  });

  it("clears the active recorder when a recovered transcription failure is scoped", async () => {
    const failedNote = {
      ...first,
      processingStatus: "failed" as const,
      lastError: "Microphone: upstream_provider_failed",
      audioSources: [
        {
          id: "audio-1",
          source: "microphone" as const,
          format: "wav" as const,
          durationMs: 1000,
          sizeBytes: 2048,
          checksum: "abc",
          createdAt: now,
        },
      ],
    };
    let recoveryFailed = false;
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [first, second],
      activeRecoveries: [recovery()],
    });
    mocks.getNote.mockImplementation(async (noteId: string) => {
      if (noteId === "note-2") return second;
      return recoveryFailed ? failedNote : first;
    });
    mocks.recoverRecording.mockImplementation(async () => {
      recoveryFailed = true;
      throw {
        code: "transcription_failed",
        message: "Microphone: The transcription provider could not process this audio.",
      };
    });

    await startRecordingOnFirstNote();
    expect(await screen.findByRole("button", { name: "Done" })).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: "Recover" }));

    await waitFor(() =>
      expect(
        screen.getByText(/Microphone: The transcription provider could not process this audio\./),
      ).toBeInTheDocument(),
    );
    expect(screen.queryByRole("button", { name: "Done" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Open recording: First note" })).toBeNull();
  });

  it("applies the finish result even when the note already sat in a terminal status", async () => {
    mocks.finishRecording.mockResolvedValue({
      note: { ...first, processingStatus: "transcribing" },
      recording: recording({ state: "ready" }),
      validation: {},
      processingStarted: true,
    });

    await startRecordingOnFirstNote();

    // note-1 is "ready" (terminal); stacking another take must still flip it
    // back to transcribing so the shimmer shows and progress events apply.
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(mocks.finishRecording).toHaveBeenCalledWith("rec-1"));

    await waitFor(() => expect(screen.getByText(/Transcribing audio/)).toBeInTheDocument());
  });

  it("clears a stale failure when Stop optimistically starts transcription", async () => {
    const failedNote = {
      ...first,
      processingStatus: "failed" as const,
      lastError: "Microphone: upstream_provider_failed",
    };
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [failedNote, second],
      activeRecoveries: [],
    });
    mocks.getNote.mockImplementation(async (noteId: string) =>
      noteId === "note-2" ? second : failedNote,
    );
    const pendingFinish = deferred<never>();
    mocks.finishRecording.mockReturnValue(pendingFinish.promise);

    await startRecordingDirectlyOnFirstNote();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(mocks.finishRecording).toHaveBeenCalledWith("rec-1"));

    expect(await screen.findByText(/Transcribing audio/)).toBeInTheDocument();
    expect(screen.queryByRole("alert", { name: "Transcription warning" })).toBeNull();
    expect(
      screen.queryByText(/The transcription provider could not process this audio\./),
    ).toBeNull();
  });

  it("clears a stale recovery error when Stop optimistically starts transcription", async () => {
    const recoverableNote = {
      ...first,
      processingStatus: "recoverable" as const,
      lastError: "Recording interrupted. Clovy saved the audio for recovery.",
    };
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [recoverableNote, second],
      activeRecoveries: [],
    });
    mocks.getNote.mockImplementation(async (noteId: string) =>
      noteId === "note-2" ? second : recoverableNote,
    );
    const pendingFinish = deferred<never>();
    mocks.finishRecording.mockReturnValue(pendingFinish.promise);

    await startRecordingDirectlyOnFirstNote();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(mocks.finishRecording).toHaveBeenCalledWith("rec-1"));

    expect(await screen.findByText(/Transcribing audio/)).toBeInTheDocument();
    expect(screen.queryByRole("alert", { name: "Transcription warning" })).toBeNull();
    expect(screen.queryByText(/Recording interrupted/)).toBeNull();
  });

  it("preserves an active warning when Stop queues another recording", async () => {
    const warningNote = {
      ...first,
      processingStatus: "transcribing" as const,
      lastError: "authorization_denied",
    };
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [warningNote, second],
      activeRecoveries: [],
    });
    mocks.getNote.mockImplementation(async (noteId: string) =>
      noteId === "note-2" ? second : warningNote,
    );
    mocks.finishRecording.mockResolvedValue({
      note: { ...warningNote, queuedRecordings: 1 },
      recording: recording({ state: "ready" }),
      validation: {},
      processingStarted: true,
    });

    await startRecordingDirectlyOnFirstNote();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(mocks.finishRecording).toHaveBeenCalledWith("rec-1"));

    await waitFor(() =>
      expect(screen.getByRole("alert", { name: "Transcription warning" })).toHaveTextContent(
        "The service is busy right now. Wait a minute, then retry.",
      ),
    );
  });

  it("refetches the full note only when processing emits done", async () => {
    const selectedNote = note({
      processingStatus: "transcribing",
      activeTab: "transcription",
      sourceTranscripts: [],
    });
    const completedNote = {
      ...selectedNote,
      processingStatus: "ready" as const,
      updatedAt: "2026-05-19T10:00:03Z",
      sourceTranscripts: [
        {
          id: "turn-1",
          text: "The completed note is visible.",
          source: "microphone" as const,
          sourceMode: "microphonePlusSystem" as const,
          startMs: 0,
          endMs: 4_000,
          turnIndex: 0,
          language: "en",
          status: "succeeded" as const,
          recordedSilence: false,
        },
      ],
    };
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [selectedNote],
      activeRecoveries: [],
    });
    let processingDone = false;
    mocks.getNote.mockImplementation(async () => (processingDone ? completedNote : selectedNote));

    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(await screen.findByRole("button", { name: /First note Preview/ }));
    await waitFor(() => expect(screen.getByText("Transcribing audio")).toBeInTheDocument());
    await waitFor(() => expect(mocks.listeners.has("note-processing-progress")).toBe(true));

    mocks.getNote.mockClear();
    await act(async () => {
      await mocks.listeners.get("note-processing-progress")?.({
        payload: {
          noteId: selectedNote.id,
          recordingSessionId: "recording-1",
          stage: "generating",
          processingStatus: "generating",
          revision: "2026-05-19T10:00:02Z",
        },
      });
    });
    expect(mocks.getNote).not.toHaveBeenCalled();
    expect(screen.queryByText("The completed note is visible.")).not.toBeInTheDocument();

    processingDone = true;
    await act(async () => {
      await mocks.listeners.get("note-processing-progress")?.({
        payload: {
          noteId: selectedNote.id,
          recordingSessionId: "recording-1",
          stage: "done",
          processingStatus: "ready",
          revision: completedNote.updatedAt,
        },
      });
    });

    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(1));
    expect(mocks.getNote).toHaveBeenCalledWith(selectedNote.id);
    expect(await screen.findByText("The completed note is visible.")).toBeInTheDocument();
  });

  it("retries a terminal hydration that fails transiently", async () => {
    const selectedNote = note({
      processingStatus: "generating",
      activeTab: "transcription",
      sourceTranscripts: [],
    });
    const completedNote = {
      ...selectedNote,
      processingStatus: "ready" as const,
      updatedAt: "2026-05-19T10:00:05Z",
      sourceTranscripts: [
        {
          id: "turn-terminal-retry",
          text: "Recovered after terminal hydration failed.",
          source: "microphone" as const,
          sourceMode: "microphonePlusSystem" as const,
          startMs: 0,
          endMs: 4_000,
          turnIndex: 0,
          language: "en",
          status: "succeeded" as const,
          recordedSilence: false,
        },
      ],
    };
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [selectedNote],
      activeRecoveries: [],
    });
    mocks.getNote.mockResolvedValue(selectedNote);

    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(await screen.findByRole("button", { name: /First note Preview/ }));
    await waitFor(() => expect(mocks.listeners.has("note-processing-progress")).toBe(true));

    mocks.getNote.mockClear();
    mocks.getNote
      .mockRejectedValueOnce(new Error("Temporary note read failure"))
      .mockResolvedValue(completedNote);
    const setTimeoutSpy = vi.spyOn(window, "setTimeout");

    try {
      await act(async () => {
        await mocks.listeners.get("note-processing-progress")?.({
          payload: {
            noteId: selectedNote.id,
            recordingSessionId: "recording-1",
            stage: "done",
            processingStatus: "ready",
            revision: completedNote.updatedAt,
          },
        });
      });

      await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(1));
      let retry: TimerHandler | undefined;
      await waitFor(() => {
        retry = setTimeoutSpy.mock.calls.find(
          ([, delay]) => delay === NOTE_PROCESSING_RECONCILE_INTERVAL_MS,
        )?.[0];
        expect(retry).toBeTypeOf("function");
      });

      await act(async () => {
        if (typeof retry === "function") retry();
      });

      await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(2));
      expect(
        await screen.findByText("Recovered after terminal hydration failed."),
      ).toBeInTheDocument();
    } finally {
      setTimeoutSpy.mockRestore();
    }
  });

  it.each([
    ["window focus", () => window.dispatchEvent(new Event("focus"))],
    ["visible document", () => document.dispatchEvent(new Event("visibilitychange"))],
  ])("recovers a dropped done event when the %s resumes", async (_surface, resume) => {
    const selectedNote = note({
      processingStatus: "transcribing",
      activeTab: "transcription",
      sourceTranscripts: [],
    });
    const completedNote = {
      ...selectedNote,
      processingStatus: "ready" as const,
      updatedAt: "2026-05-19T10:00:03Z",
      sourceTranscripts: [
        {
          id: "turn-focus-recovery",
          text: "Recovered after the done event was dropped.",
          source: "microphone" as const,
          sourceMode: "microphonePlusSystem" as const,
          startMs: 0,
          endMs: 4_000,
          turnIndex: 0,
          language: "en",
          status: "succeeded" as const,
          recordedSilence: false,
        },
      ],
    };
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [selectedNote],
      activeRecoveries: [],
    });
    let processingDone = false;
    mocks.getNote.mockImplementation(async () => (processingDone ? completedNote : selectedNote));

    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(await screen.findByRole("button", { name: /First note Preview/ }));
    await waitFor(() => expect(screen.getByText("Transcribing audio")).toBeInTheDocument());

    mocks.getNote.mockClear();
    processingDone = true;
    await act(async () => resume());

    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(1));
    expect(mocks.getNote).toHaveBeenCalledWith(selectedNote.id);
    expect(
      await screen.findByText("Recovered after the done event was dropped."),
    ).toBeInTheDocument();
  });

  it("does not install the slow backstop when no note is actively processing", async () => {
    const setIntervalSpy = vi.spyOn(window, "setInterval");
    try {
      render(<App />);
      await waitFor(() => expect(mocks.listeners.has("note-processing-progress")).toBe(true));
      expect(
        setIntervalSpy.mock.calls.some(
          ([, delay]) => delay === NOTE_PROCESSING_RECONCILE_INTERVAL_MS,
        ),
      ).toBe(false);
    } finally {
      setIntervalSpy.mockRestore();
    }
  });

  it("recovers a dropped done event through the slow processing-only backstop", async () => {
    const selectedNote = note({
      processingStatus: "generating",
      activeTab: "transcription",
      sourceTranscripts: [],
    });
    const completedNote = {
      ...selectedNote,
      processingStatus: "ready" as const,
      updatedAt: "2026-05-19T10:00:04Z",
      sourceTranscripts: [
        {
          id: "turn-backstop-recovery",
          text: "Recovered by the slow processing backstop.",
          source: "system" as const,
          sourceMode: "microphonePlusSystem" as const,
          startMs: 0,
          endMs: 4_000,
          turnIndex: 0,
          language: "en",
          status: "succeeded" as const,
          recordedSilence: false,
        },
      ],
    };
    mocks.bootstrapApp.mockResolvedValue({
      folders: [],
      notes: [selectedNote],
      activeRecoveries: [],
    });
    let processingDone = false;
    mocks.getNote.mockImplementation(async () => (processingDone ? completedNote : selectedNote));
    const setIntervalSpy = vi.spyOn(window, "setInterval");

    try {
      render(<App />);
      await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
      await userEvent.click(await screen.findByRole("button", { name: /First note Preview/ }));

      let backstop: TimerHandler | undefined;
      await waitFor(() => {
        backstop = setIntervalSpy.mock.calls.find(
          ([, delay]) => delay === NOTE_PROCESSING_RECONCILE_INTERVAL_MS,
        )?.[0];
        expect(backstop).toBeTypeOf("function");
      });

      mocks.getNote.mockClear();
      processingDone = true;
      await act(async () => {
        if (typeof backstop === "function") backstop();
      });

      await waitFor(() => expect(mocks.getNote).toHaveBeenCalledTimes(1));
      expect(mocks.getNote).toHaveBeenCalledWith(selectedNote.id);
      expect(
        await screen.findByText("Recovered by the slow processing backstop."),
      ).toBeInTheDocument();
    } finally {
      setIntervalSpy.mockRestore();
    }
  });

  it("keeps retry failures scoped to the failed note", async () => {
    mocks.getNote.mockImplementation(async (noteId: string) =>
      noteId === "note-2"
        ? second
        : {
            ...first,
            processingStatus: "failed",
            lastError: "Network unreachable",
            audio: {
              id: "audio-1",
              format: "wav",
              durationMs: 1000,
              sizeBytes: 2048,
              checksum: "abc",
              createdAt: now,
            },
          },
    );
    mocks.retryProcessing.mockRejectedValue({
      code: "clovy_api_response_invalid",
      message: "The processing service returned an invalid response.",
    });

    const { container } = render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));

    // The app launches on the agent view; open the note from the Meetings
    // list so the editor (and its failure banner) is on screen.
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(screen.getByRole("button", { name: /First note Preview/ }));

    await userEvent.click(screen.getByRole("button", { name: /Retry/ }));

    await waitFor(() =>
      expect(
        screen.getByText(/The processing service returned an invalid response\./),
      ).toBeInTheDocument(),
    );
    expect(container.querySelector(".note-failure-banner")).not.toBeNull();
    expect(container.querySelector(".error-banner")).toBeNull();
    // The banner releases its busy gate so the user can try again.
    await waitFor(() => expect(screen.getByRole("button", { name: /Retry/ })).toBeEnabled());

    await userEvent.click(screen.getByRole("button", { name: "Dictation" }));
    expect(container.querySelector(".note-failure-banner")).toBeNull();
    expect(container.querySelector(".error-banner")).toBeNull();
  });

  it("shows processing immediately after retry starts", async () => {
    const failedNote = {
      ...first,
      activeTab: "notes" as const,
      processingStatus: "failed" as const,
      retryRecordingSessionId: "recording-to-retry",
      lastError: "The processing service returned an invalid response.",
      audio: {
        id: "audio-1",
        format: "wav",
        durationMs: 1000,
        sizeBytes: 2048,
        checksum: "abc",
        createdAt: now,
      },
    };
    const retryingNote = {
      ...failedNote,
      processingStatus: "transcribing" as const,
      lastError: undefined,
    };
    mocks.getNote.mockImplementation(async (noteId: string) =>
      noteId === "note-2" ? second : failedNote,
    );
    mocks.retryProcessing.mockResolvedValue(retryingNote);
    mocks.updateNote.mockImplementation(async (input) => ({
      ...retryingNote,
      ...input,
    }));

    const { container } = render(<App />);
    await waitFor(() => expect(mocks.getNote).toHaveBeenCalledWith("note-1"));
    await userEvent.click(await screen.findByRole("button", { name: "Meeting notes" }));
    await userEvent.click(screen.getByRole("button", { name: /First note Preview/ }));

    await userEvent.click(screen.getByRole("button", { name: /Retry/ }));

    await waitFor(() =>
      expect(mocks.retryProcessing).toHaveBeenCalledWith("note-1", "recording-to-retry"),
    );

    await waitFor(() => expect(screen.getByText(/Transcribing audio/)).toBeInTheDocument());
    expect(container.querySelector(".note-failure-banner")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Transcription" }));
    await waitFor(() =>
      expect(mocks.patchNote).toHaveBeenCalledWith("note-1", {
        activeTab: "transcription",
      }),
    );
  });
});
