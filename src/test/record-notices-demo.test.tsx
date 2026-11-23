import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RECORD_NOTICES_DEMO_SESSION_ID } from "../app/processing-demo-ids";
import { NoteEditor } from "../components/note-editor/NoteEditor";
import { type RecordNoticesDemoApi, registerRecordNoticesDemo } from "../lib/record-notices-demo";
import type { NoteDto, RecordingStatusDto } from "../lib/tauri";

const now = "2026-05-19T10:00:00Z";

function note(overrides: Partial<NoteDto> = {}): NoteDto {
  return {
    id: "note-1",
    title: "Generated note",
    preview: "Preview",
    processingStatus: "ready",
    folderIds: [],
    createdAt: now,
    updatedAt: now,
    generatedContent: "## Section one\n\n- First point",
    activeTab: "notes",
    ...overrides,
  };
}

const editorProps = {
  folders: [],
  sourceMode: "microphonePlusSystem" as const,
  onTitleChange: vi.fn(),
  onContentChange: vi.fn(),
  onFlushNote: vi.fn(),
  onSourceModeChange: vi.fn(),
  onEnableSystemAudio: vi.fn(),
  onEnableMicrophone: vi.fn(),
  microphoneBlocked: false,
  onStartRecording: vi.fn(),
  onPauseRecording: vi.fn(),
  onResumeRecording: vi.fn(),
  onFinishRecording: vi.fn(),
  onRetry: vi.fn(),
  onRecoverRecording: vi.fn(),
  onDiscardRecording: vi.fn(),
  onAssignFolder: vi.fn(),
  onRemoveFolder: vi.fn(),
  onCreateAndAssignFolder: vi.fn(),
  onNavigateToFolder: vi.fn(),
  onTabChange: vi.fn(),
};

// A harness that stands in for App's side of the driver: it records the latest
// synthetic status / pins so tests can feed them into a real NoteEditor, just
// as App does. getSelectedNoteId returns a note by default so no seeding fires.
function setup(options: { selectedNoteId?: string | undefined; hasRealRecording?: boolean } = {}) {
  const seedNote = vi.fn<(note: NoteDto) => void>();
  const setConsentPinned = vi.fn<(pinned: boolean) => void>();
  const setMicOverride = vi.fn<(blocked: boolean | null) => void>();
  let status: RecordingStatusDto | null = null;
  const setStatus = vi.fn((next: RecordingStatusDto | null) => {
    status = next;
  });
  const api: RecordNoticesDemoApi = registerRecordNoticesDemo({
    seedNote,
    setStatus,
    setConsentPinned,
    setMicOverride,
    getSelectedNoteId: () => ("selectedNoteId" in options ? options.selectedNoteId : "note-1"),
    hasRealRecording: () => options.hasRealRecording ?? false,
  });
  const invoke = (command?: string, arg?: string) =>
    (window as unknown as Record<string, (c?: string, a?: string) => string>).__recordNoticesDemo(
      command,
      arg,
    );
  return {
    api,
    invoke,
    seedNote,
    setConsentPinned,
    setMicOverride,
    setStatus,
    latestStatus: () => status,
  };
}

describe("registerRecordNoticesDemo", () => {
  afterEach(() => {
    vi.useRealTimers();
    delete (window as unknown as Record<string, unknown>).__recordNoticesDemo;
  });

  it("returns help for no or unknown command", () => {
    const { api, invoke } = setup();
    expect(invoke()).toContain("Recorder notices demo");
    expect(invoke("bogus")).toContain('__recordNoticesDemo("consent")');
    api.dispose();
  });

  it("parks the consent reminder under the sentinel session the poll skips", () => {
    vi.useFakeTimers();
    const { api, invoke, setConsentPinned, setMicOverride, latestStatus } = setup();

    invoke("consent");

    // Pinned open, mic notice off, and a live recording status with no warnings.
    expect(setConsentPinned).toHaveBeenLastCalledWith(true);
    expect(setMicOverride).toHaveBeenLastCalledWith(null);
    const status = latestStatus();
    expect(status).not.toBeNull();
    // The exact sentinel App's status poll and pause/finish handlers skip, so no
    // backend call fires for the demo session.
    expect(status?.sessionId).toBe(RECORD_NOTICES_DEMO_SESSION_ID);
    expect(status?.warnings ?? []).toHaveLength(0);

    // The reminder shows immediately when pinned — no reveal timer to advance.
    render(
      <NoteEditor
        {...editorProps}
        note={note()}
        recordingStatus={status ?? undefined}
        consentReminderPinned
      />,
    );
    expect(screen.getByRole("status", { name: "Recording consent reminder" })).toBeInTheDocument();

    // ...and it is never auto-hidden while pinned.
    act(() => {
      vi.advanceTimersByTime(6000);
    });
    expect(screen.getByRole("status", { name: "Recording consent reminder" })).toBeInTheDocument();
    api.dispose();
  });

  it("parks a source warning that suppresses the consent reminder", () => {
    const { api, invoke, setConsentPinned, latestStatus } = setup();

    invoke("warning");

    expect(setConsentPinned).toHaveBeenLastCalledWith(false);
    const status = latestStatus();
    expect(status?.warnings).toHaveLength(1);
    expect(status?.warnings?.[0].message).toContain("Microphone input stopped unexpectedly");

    render(<NoteEditor {...editorProps} note={note()} recordingStatus={status ?? undefined} />);
    expect(screen.getByText(/Microphone input stopped unexpectedly/)).toBeInTheDocument();
    expect(
      screen.queryByText("Make sure everyone has agreed to be recorded."),
    ).not.toBeInTheDocument();
    api.dispose();
  });

  it("overrides the warning message from the second argument", () => {
    const { api, invoke, latestStatus } = setup();
    invoke("warning", "System audio capture stopped unexpectedly.");
    expect(latestStatus()?.warnings?.[0].message).toBe(
      "System audio capture stopped unexpectedly.",
    );
    api.dispose();
  });

  it("parks the mic-blocked notice with no active recording", () => {
    const { api, invoke, setStatus, setMicOverride, setConsentPinned } = setup();

    invoke("mic");

    // No recording is pushed (mutually exclusive with the mic notice), the mic
    // override is forced on, and consent pinning is released.
    expect(setStatus).toHaveBeenLastCalledWith(null);
    expect(setMicOverride).toHaveBeenLastCalledWith(true);
    expect(setConsentPinned).toHaveBeenLastCalledWith(false);

    render(
      <NoteEditor {...editorProps} note={note()} microphoneBlocked recordingStatus={undefined} />,
    );
    expect(screen.getByText(/Microphone access is blocked/)).toBeInTheDocument();
    api.dispose();
  });

  it("seeds a note when none is selected", () => {
    const { api, invoke, seedNote } = setup({ selectedNoteId: undefined });
    invoke("consent");
    expect(seedNote).toHaveBeenCalledTimes(1);
    expect(seedNote.mock.calls[0][0].processingStatus).toBe("recording");
    api.dispose();
  });

  it("clear restores real state", () => {
    const { api, invoke, setStatus, setConsentPinned, setMicOverride } = setup();
    invoke("consent");
    setStatus.mockClear();
    setConsentPinned.mockClear();
    setMicOverride.mockClear();

    expect(invoke("clear")).toContain("back to real state");
    expect(setStatus).toHaveBeenLastCalledWith(null);
    expect(setConsentPinned).toHaveBeenLastCalledWith(false);
    expect(setMicOverride).toHaveBeenLastCalledWith(null);
    api.dispose();
  });

  it("refuses state-mutating commands while a real recording is active", () => {
    const { api, invoke, setStatus, setConsentPinned, setMicOverride, latestStatus } = setup({
      hasRealRecording: true,
    });

    const refusal = "A real recording is in progress. Finish it before running the demo.";
    for (const command of ["consent", "warning", "mic"] as const) {
      expect(invoke(command)).toBe(refusal);
    }

    // None of them touched the reducer status, consent pin, or mic override.
    expect(setStatus).not.toHaveBeenCalled();
    expect(setConsentPinned).not.toHaveBeenCalled();
    expect(setMicOverride).not.toHaveBeenCalled();
    expect(latestStatus()).toBeNull();
    api.dispose();
  });

  it("clear does not clear a real recording", () => {
    const { api, invoke, setStatus, setConsentPinned, setMicOverride } = setup({
      hasRealRecording: true,
    });

    expect(invoke("clear")).toBe(
      "A real recording is in progress. Finish it before running the demo.",
    );
    expect(setStatus).not.toHaveBeenCalled();
    expect(setConsentPinned).not.toHaveBeenCalled();
    expect(setMicOverride).not.toHaveBeenCalled();
    api.dispose();
  });

  it("dispose removes the window hook and stops the ticker", () => {
    vi.useFakeTimers();
    const { api, invoke, setStatus } = setup();
    invoke("consent");
    api.dispose();
    setStatus.mockClear();
    // The ticker interval must be cleared on dispose — no further status pushes.
    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(setStatus).not.toHaveBeenCalled();
    expect((window as unknown as Record<string, unknown>).__recordNoticesDemo).toBeUndefined();
  });
});
