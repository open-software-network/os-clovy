import type { RecordNoticesDemoApi } from "../lib/record-notices-demo";
import type { SidebarView } from "../components/sidebar/Sidebar";
import type { Tab } from "./tabs/tabs";
import type { NoteDto, RecordingStatusDto } from "../lib/tauri";
import type { RecordingSourceMode } from "../lib/tauri";
import type { NotesAction, NotesState } from "./state/app-state";
import type * as React from "react";

export type UseRecordingControlsDependencies = {
  activeViewRef: React.MutableRefObject<SidebarView>;
  appBlocked: boolean;
  bootstrapped: boolean;
  crossPartitionRecordingNoteIdRef: React.MutableRefObject<string | undefined>;
  dispatch: React.Dispatch<NotesAction>;
  finishingSessionsRef: React.MutableRefObject<Set<string>>;
  handleStartAgentRecording: (
    requestedSourceMode: RecordingSourceMode,
    target?: { noteId: string; profile: string; requestId?: string },
  ) => Promise<NoteDto>;
  recordNoticesDemoRef: React.MutableRefObject<RecordNoticesDemoApi | null>;
  recordingNoteIdRef: React.MutableRefObject<string | undefined>;
  recordingStatusRef: React.MutableRefObject<RecordingStatusDto | undefined>;
  selectedNote: NoteDto | undefined;
  setActiveView: React.Dispatch<React.SetStateAction<SidebarView>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setFolderReturnTarget: React.Dispatch<
    React.SetStateAction<{ noteId: string; label: string } | undefined>
  >;
  setOriginAllNotes: React.Dispatch<React.SetStateAction<boolean>>;
  setOriginFolderId: React.Dispatch<React.SetStateAction<string | undefined>>;
  setRecordingNote: (noteId: string | undefined) => void;
  setTabs: React.Dispatch<React.SetStateAction<Tab[]>>;
  state: NotesState;
  tabsRef: React.MutableRefObject<Tab[]>;
};
