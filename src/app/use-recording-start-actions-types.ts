import type { SidebarView } from "../components/sidebar/Sidebar";
import type { NoteDto, RecordingStatusDto } from "../lib/tauri";
import type { RecordingSourceMode, RecordingSourceReadinessDto } from "../lib/tauri";
import type { NotesAction } from "./state/app-state";
import type * as React from "react";

export type UseRecordingStartActionsDependencies = {
  activeViewRef: React.MutableRefObject<SidebarView>;
  appBlocked: boolean;
  bootstrapped: boolean;
  calendarContextNotePartitionsRef: React.MutableRefObject<Map<string, string>>;
  calendarContextNoteUpdatesRef: React.MutableRefObject<Map<string, NoteDto>>;
  crossPartitionRecordingNoteIdRef: React.MutableRefObject<string | undefined>;
  dispatch: React.Dispatch<NotesAction>;
  handleEmptyNotesAfterDelete: () => void;
  pendingCalendarContextAdoptionsRef: React.MutableRefObject<Set<string>>;
  recordingNoteIdRef: React.MutableRefObject<string | undefined>;
  recordingStartInFlightRef: React.MutableRefObject<boolean>;
  recordingStatusRef: React.MutableRefObject<RecordingStatusDto | undefined>;
  selectedNoteId: string | undefined;
  selectedNoteIdRef: React.MutableRefObject<string | undefined>;
  setActiveView: React.Dispatch<React.SetStateAction<SidebarView>>;
  setCheckingSourceReadiness: React.Dispatch<React.SetStateAction<boolean>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setFolderReturnTarget: React.Dispatch<
    React.SetStateAction<{ noteId: string; label: string } | undefined>
  >;
  setOriginAllNotes: React.Dispatch<React.SetStateAction<boolean>>;
  setOriginFolderId: React.Dispatch<React.SetStateAction<string | undefined>>;
  setRecordingNote: (noteId: string | undefined) => void;
  setSourceReadiness: React.Dispatch<React.SetStateAction<RecordingSourceReadinessDto | undefined>>;
  sourceMode: RecordingSourceMode;
};
