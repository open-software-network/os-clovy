import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  checkRecordingSourceReadiness,
  completeNoteSaveFlush,
  downloadNoteAudio,
  finishRecording,
  getNote,
  clovyPersona,
  patchNote,
  recoverRecording,
  retryProcessing,
  startRecording,
  startMeetingRecording,
  detectMeetingWindow,
  setClovyPersona,
  updateNote,
} from "../lib/tauri";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

describe("Tauri command contracts", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue({});
  });

  it("wraps note retrieval and updates in request payloads", async () => {
    await getNote("note-1");
    await updateNote({
      noteId: "note-1",
      title: "Updated",
      editedContent: "Manual notes",
      activeTab: "transcription",
    });

    expect(mocks.invoke).toHaveBeenNthCalledWith(1, "get_note", {
      request: { noteId: "note-1" },
    });
    expect(mocks.invoke).toHaveBeenNthCalledWith(2, "update_note", {
      request: {
        noteId: "note-1",
        title: "Updated",
        editedContent: "Manual notes",
        activeTab: "transcription",
      },
    });
  });

  it("opts autosave into the additive patch response and acknowledges shutdown flushes", async () => {
    await patchNote("note-1", {
      title: "Updated",
      editedContent: "Manual notes",
    });
    await completeNoteSaveFlush("flush-1");

    expect(mocks.invoke).toHaveBeenNthCalledWith(1, "update_note", {
      request: {
        noteId: "note-1",
        title: "Updated",
        editedContent: "Manual notes",
        patchOnly: true,
      },
    });
    expect(mocks.invoke).toHaveBeenNthCalledWith(2, "complete_note_save_flush", {
      request: { requestId: "flush-1" },
    });
  });

  it("downloads note audio through a note-scoped request", async () => {
    const response = {
      path: "/tmp/First note audio.wav",
      fileName: "First note audio.wav",
      sourceCount: 1,
    };
    mocks.invoke.mockResolvedValue(response);

    await expect(downloadNoteAudio("note-1")).resolves.toEqual(response);
    expect(mocks.invoke).toHaveBeenCalledWith("download_note_audio", {
      request: { noteId: "note-1" },
    });
  });

  it("sends recording lifecycle commands with stable request shapes", async () => {
    await checkRecordingSourceReadiness("microphonePlusSystem");
    await startRecording("note-1", "microphonePlusSystem");
    await finishRecording("session-1");

    expect(mocks.invoke).toHaveBeenNthCalledWith(1, "check_recording_source_readiness", {
      request: { sourceMode: "microphonePlusSystem" },
    });
    expect(mocks.invoke).toHaveBeenNthCalledWith(2, "start_recording", {
      request: { noteId: "note-1", sourceMode: "microphonePlusSystem" },
    });
    expect(mocks.invoke).toHaveBeenNthCalledWith(3, "finish_recording", {
      request: { sessionId: "session-1" },
    });
  });

  it("detects meeting windows and adds video only to opted-in starts", async () => {
    await detectMeetingWindow();
    await startRecording("note-video", "microphoneOnly", undefined, undefined, { windowId: 42 });
    await startMeetingRecording("request-video", "microphonePlusSystem", { windowId: 42 });
    expect(mocks.invoke).toHaveBeenNthCalledWith(1, "detect_meeting_window");
    expect(mocks.invoke).toHaveBeenNthCalledWith(2, "start_recording", {
      request: { noteId: "note-video", sourceMode: "microphoneOnly", video: { windowId: 42 } },
    });
    expect(mocks.invoke).toHaveBeenNthCalledWith(3, "start_meeting_recording", {
      request: {
        requestId: "request-video",
        sourceMode: "microphonePlusSystem",
        video: { windowId: 42 },
      },
    });
  });

  it("keeps retry and recovery commands authoritative", async () => {
    await retryProcessing("note-1");
    await retryProcessing("note-1", "recording-2");
    await recoverRecording("session-1", "validate");
    await recoverRecording("session-2", "discard");

    expect(mocks.invoke).toHaveBeenNthCalledWith(1, "retry_processing", {
      request: { noteId: "note-1", step: "all" },
    });
    expect(mocks.invoke).toHaveBeenNthCalledWith(2, "retry_processing", {
      request: {
        noteId: "note-1",
        step: "all",
        recordingSessionId: "recording-2",
      },
    });
    expect(mocks.invoke).toHaveBeenNthCalledWith(3, "recover_recording", {
      request: { sessionId: "session-1", action: "validate" },
    });
    expect(mocks.invoke).toHaveBeenNthCalledWith(4, "recover_recording", {
      request: { sessionId: "session-2", action: "discard" },
    });
  });

  it("loads and saves Clovy's persona through stable native commands", async () => {
    await clovyPersona();
    await setClovyPersona({
      area: "thinking",
      voice: 45,
      detail: 90,
      initiative: 75,
      humor: 20,
    });

    expect(mocks.invoke).toHaveBeenNthCalledWith(1, "clovy_persona");
    expect(mocks.invoke).toHaveBeenNthCalledWith(2, "set_clovy_persona", {
      request: {
        area: "thinking",
        voice: 45,
        detail: 90,
        initiative: 75,
        humor: 20,
      },
    });
  });
});
