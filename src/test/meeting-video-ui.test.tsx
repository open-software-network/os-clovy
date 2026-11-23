import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NoteEditor } from "../components/note-editor/NoteEditor";
import { setInterfaceLocale, t } from "../i18n";
import { resetInterfaceLocaleForTests } from "../i18n/locale";
import type { NoteDto } from "../lib/tauri";

const mocks = vi.hoisted(() => ({ detect: vi.fn() }));
vi.mock("../lib/tauri", () => ({
  detectMeetingWindow: mocks.detect,
  listRecordingMarks: vi.fn().mockResolvedValue([]),
  getRecordingAssistanceStatus: vi
    .fn()
    .mockResolvedValue({ epoch: 0, enabled: false, hasRecentContext: false }),
}));
vi.mock("../components/note-editor/NotePreview", () => ({
  NotePreview: () => <div>Note body</div>,
}));
const meeting = {
  windowId: 42,
  appName: "Chrome",
  bundleId: "com.google.Chrome",
  title: "Meet - Product review",
};
const note: NoteDto = {
  id: "video-note",
  title: "Review",
  preview: "",
  processingStatus: "ready",
  folderIds: [],
  createdAt: "2026-10-28T10:00:00Z",
  updatedAt: "2026-10-28T10:00:00Z",
};
const noop = () => {};
const props = {
  note,
  folders: [],
  sourceMode: "microphoneOnly" as const,
  onTitleChange: noop,
  onContentChange: noop,
  onFlushNote: noop,
  onSourceModeChange: noop,
  onEnableSystemAudio: noop,
  onEnableMicrophone: noop,
  microphoneBlocked: false,
  onStartRecording: vi.fn(),
  onPauseRecording: noop,
  onResumeRecording: noop,
  onFinishRecording: noop,
  onRetry: noop,
  onRecoverRecording: noop,
  onDiscardRecording: noop,
  onAssignFolder: noop,
  onRemoveFolder: noop,
  onCreateAndAssignFolder: noop,
  onTabChange: noop,
};
const originalPlatform = navigator.platform;
beforeEach(() => {
  mocks.detect.mockReset().mockResolvedValue(null);
  props.onStartRecording.mockReset();
  Object.defineProperty(navigator, "platform", { configurable: true, value: "MacIntel" });
  resetInterfaceLocaleForTests();
});
afterEach(() => {
  Object.defineProperty(navigator, "platform", { configurable: true, value: originalPlatform });
  resetInterfaceLocaleForTests();
});

describe("meeting video options", () => {
  it("clears stale video after recording auto-closes options before a fresh lookup completes", async () => {
    const freshWindow = { ...meeting, windowId: 99, title: "Meet - New review" };
    let resolveFresh: ((window: typeof meeting) => void) | undefined;
    mocks.detect.mockResolvedValueOnce(meeting).mockReturnValueOnce(
      new Promise<typeof meeting>((resolve) => {
        resolveFresh = resolve;
      }),
    );
    const { rerender } = render(<NoteEditor {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Recording options" }));
    fireEvent.click(await screen.findByRole("switch", { name: "Record meeting video" }));
    rerender(
      <NoteEditor
        {...props}
        recordingStatus={{
          sessionId: "other-start",
          noteId: note.id,
          state: "recording",
          elapsedMs: 8000,
          level: { peak: 0.5, rms: 0.2, recentPeaks: [0.1, 0.3] },
          silenceWarning: false,
          bytesWritten: 4096,
        }}
      />,
    );
    rerender(<NoteEditor {...props} />);
    fireEvent.click(await screen.findByRole("button", { name: "Recording options" }));
    expect(mocks.detect).toHaveBeenCalledTimes(2);
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    expect(props.onStartRecording).toHaveBeenCalledWith(undefined);
    expect(screen.queryByText("Chrome: Meet - Product review")).toBeNull();
    await act(async () => {
      resolveFresh?.(freshWindow);
    });
    const freshToggle = await screen.findByRole("switch", { name: "Record meeting video" });
    expect(freshToggle).toHaveAttribute("aria-checked", "false");
    expect(screen.getByText("Chrome: Meet - New review")).toBeInTheDocument();
    fireEvent.click(freshToggle);
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    expect(props.onStartRecording).toHaveBeenLastCalledWith({ windowId: 99 });
    expect(mocks.detect).toHaveBeenCalledTimes(2);
  });

  it("offers no video switch when no meeting window is detected", async () => {
    render(<NoteEditor {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Recording options" }));
    await waitFor(() => expect(mocks.detect).toHaveBeenCalledOnce());
    expect(screen.queryByRole("switch", { name: "Record meeting video" })).toBeNull();
  });
  it("shows the selected window and starts audio without waiting for another detection", async () => {
    mocks.detect.mockResolvedValueOnce(meeting).mockReturnValueOnce(new Promise<null>(() => {}));
    render(<NoteEditor {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Recording options" }));
    const toggle = await screen.findByRole("switch", { name: "Record meeting video" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(screen.getByText("Chrome: Meet - Product review")).toBeInTheDocument();
    expect(toggle).toHaveAccessibleDescription("Chrome: Meet - Product review");
    fireEvent.click(toggle);
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    expect(props.onStartRecording).toHaveBeenCalledWith({ windowId: 42 });
    expect(mocks.detect).toHaveBeenCalledOnce();
  });
  it("clears video when options collapse and starts audio immediately without another detection", async () => {
    mocks.detect.mockResolvedValue(meeting);
    render(<NoteEditor {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Recording options" }));
    fireEvent.click(await screen.findByRole("switch", { name: "Record meeting video" }));
    fireEvent.click(screen.getByRole("button", { name: "Recording options" }));
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    expect(props.onStartRecording).toHaveBeenCalledWith(undefined);
    expect(mocks.detect).toHaveBeenCalledOnce();
    expect(screen.queryByText("Chrome: Meet - Product review")).toBeNull();
  });

  it("detects a fresh window with video off when options reopen", async () => {
    const freshWindow = { ...meeting, windowId: 99, title: "Meet - New review" };
    let resolveFresh: ((window: typeof meeting) => void) | undefined;
    mocks.detect.mockResolvedValueOnce(meeting).mockReturnValueOnce(
      new Promise<typeof meeting>((resolve) => {
        resolveFresh = resolve;
      }),
    );
    render(<NoteEditor {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Recording options" }));
    fireEvent.click(await screen.findByRole("switch", { name: "Record meeting video" }));
    fireEvent.click(screen.getByRole("button", { name: "Recording options" }));
    fireEvent.click(screen.getByRole("button", { name: "Recording options" }));
    expect(mocks.detect).toHaveBeenCalledTimes(2);
    expect(screen.queryByText("Chrome: Meet - Product review")).toBeNull();
    await act(async () => {
      resolveFresh?.(freshWindow);
    });
    expect(await screen.findByRole("switch", { name: "Record meeting video" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(screen.getByText("Chrome: Meet - New review")).toBeInTheDocument();
  });

  it("keeps audio start available when detection is denied", async () => {
    mocks.detect.mockRejectedValue({ code: "permission_denied" });
    render(<NoteEditor {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Recording options" }));
    await waitFor(() => expect(mocks.detect).toHaveBeenCalledOnce());
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    expect(props.onStartRecording).toHaveBeenCalledWith(undefined);
  });
  it("never detects or offers video on Windows", () => {
    Object.defineProperty(navigator, "platform", { configurable: true, value: "Win32" });
    render(<NoteEditor {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Recording options" }));
    expect(screen.queryByRole("switch", { name: "Record meeting video" })).toBeNull();
    expect(mocks.detect).not.toHaveBeenCalled();
  });
});

for (const locale of ["en", "pt-BR"] as const) {
  describe(`video analysis notices in ${locale}`, () => {
    const outcomes = [
      "analyzed",
      "engine_unsupported",
      "cli_unsupported",
      "skill_missing",
      "tools_missing",
      "permission_denied",
      "capture_failed",
      "analysis_failed",
      "analysis_timeout",
      "video_missing",
    ] as const;
    for (const outcome of outcomes) {
      it(`renders persisted ${outcome}`, () => {
        act(() => setInterfaceLocale(locale));
        render(<NoteEditor {...props} note={{ ...note, videoAnalysis: outcome }} />);
        if (outcome === "engine_unsupported") {
          expect(
            screen.getByRole("status", { name: t("notes.editor.video.status") }),
          ).toHaveTextContent(
            locale === "pt-BR"
              ? "Vídeo não analisado: o motor de Notas atual não analisa vídeo."
              : "Video not analyzed: the current Notes engine does not analyze video.",
          );
        }
        expect(
          screen.getByRole("status", { name: t("notes.editor.video.status") }),
        ).toHaveTextContent(t(`notes.editor.video.${outcome}`));
      });
    }
  });
}
