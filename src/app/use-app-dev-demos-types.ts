import type { SidebarView } from "../components/sidebar/Sidebar";
import type { LiveTranscriptEventDto } from "../lib/tauri";
import type { RecordingStatusDto } from "../lib/tauri";
import type { NotesAction } from "./state/app-state";
import type * as React from "react";

export type UseAppDevDemosDependencies = {
  dispatch: React.Dispatch<NotesAction>;
  getSelectedNoteId: () => string | undefined;
  recordingStatusRef: React.MutableRefObject<RecordingStatusDto | undefined>;
  setActiveView: React.Dispatch<React.SetStateAction<SidebarView>>;
  setLiveTranscriptEvents: React.Dispatch<React.SetStateAction<LiveTranscriptEventDto[]>>;
  setRecordingNote: (noteId: string | undefined) => void;
};
