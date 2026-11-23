import type { NotesListHandle } from "../components/notes-list/NotesList";
import type { SettingsTab } from "../components/settings/AppSettings";
import type { SidebarView } from "../components/sidebar/Sidebar";
import type { TabNav } from "./tabs/tabs";
import type {
  FinishRecordingResponse,
  FolderDto,
  LiveTranscriptEventDto,
  MeetingVideoRequest,
  NoteDto,
  NoteEditablePatch,
  RecoverableRecordingDto,
} from "../lib/tauri";
import type { RecordingSourceMode, RecordingSourceReadinessDto } from "../lib/tauri";
import type { NotesAction, NotesState } from "./state/app-state";
import type * as React from "react";

export type RenderAppWorkspaceDependencies = {
  checkingForUpdates: boolean;
  // Set only in a release build: the manual update check from Settings.
  onCheckForUpdates?: () => void;
  accessibilityStatus: string | undefined;
  activeView: SidebarView;
  changeSettingsTab: (tab: SettingsTab) => void;
  checkingSourceReadiness: boolean;
  dispatch: React.Dispatch<NotesAction>;
  folderReturnTarget: { noteId: string; label: string } | undefined;
  handleCreateFolder: (name: string, description?: string) => Promise<FolderDto | undefined>;
  handleCreateNote: (folderId?: string | null) => Promise<void>;
  handleDeleteFolder: (folderId: string) => Promise<void>;
  handleDeleteNote: (noteId: string) => Promise<void>;
  handleDeleteNotes: (noteIds: string[]) => Promise<void>;
  handleEnableAccessibility: () => void;
  handleEnableMicrophone: () => void;
  handleEnableSystemAudio: () => void;
  handleFinishRecording: (
    sessionId: string,
    options?: { rethrow?: boolean },
  ) => Promise<FinishRecordingResponse | undefined>;
  handleKeepRecordingAfterMeetingEnd: (sessionId: string) => void;
  handleFlushNote: (noteId: string) => Promise<void>;
  handleFoldersImported: (folders: FolderDto[]) => void;
  handlePauseRecording: (sessionId: string) => Promise<boolean>;
  handleRecovery: (sessionId: string, action: "validate" | "discard") => void;
  handleRemoveNoteFromFolder: (
    noteId: string,
    folderId: string,
    options?: { rethrow?: boolean },
  ) => Promise<void>;
  handleRenameFolder: (folderId: string, name: string, description?: string) => Promise<void>;
  handleResumeRecording: (sessionId: string) => Promise<void>;
  handleSaveNoteNow: (noteId: string, patch: NoteEditablePatch) => Promise<void>;
  handleReturnToNote: (noteId: string) => Promise<void>;
  handleSelectFolder: (folderId?: string) => void;
  handleSelectNote: (noteId: string) => Promise<void>;
  handleSelectNoteFromAllNotes: (noteId: string) => Promise<void>;
  handleSelectNoteFromFolder: (noteId: string, folderId: string) => Promise<void>;
  handleSetNoteFolder: (
    noteId: string,
    folderId: string,
    options?: { rethrow?: boolean },
  ) => Promise<void>;
  handleSourceModeChange: (next: RecordingSourceMode) => void;
  handleStartRecording: (video?: MeetingVideoRequest) => Promise<void>;
  handleStopNowAfterMeetingEnd: (sessionId: string) => void;
  handleUpdateNote: (noteId: string, patch: NoteEditablePatch) => void;
  memoryFolderFilter: string | undefined;
  meetingEndCountdown: { sessionId: string; secondsRemaining: number } | null;
  microphoneBlocked: boolean;
  microphoneStatus: string | undefined;
  noteDetailScrollRef: React.MutableRefObject<HTMLDivElement | null>;
  noteToolbarActions: JSX.Element | null;
  notesListRef: React.MutableRefObject<NotesListHandle | null>;
  openMemorySettings: (folderId?: string) => void;
  openTab: (nav: TabNav) => void;
  originAllNotes: boolean;
  originFolder: FolderDto | undefined;
  recordNoticesConsentPinned: boolean;
  recordingNoteId: string | undefined;
  selectedNote: NoteDto | undefined;
  selectedNoteId: string | undefined;
  selectedNoteLiveTranscript: LiveTranscriptEventDto[];
  selectedRecovery: RecoverableRecordingDto | undefined;
  setActiveView: React.Dispatch<React.SetStateAction<SidebarView>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setFolderReturnTarget: React.Dispatch<
    React.SetStateAction<{ noteId: string; label: string } | undefined>
  >;
  setMoveDialogNoteIds: React.Dispatch<React.SetStateAction<string[] | null>>;
  setOriginAllNotes: React.Dispatch<React.SetStateAction<boolean>>;
  setOriginFolderId: React.Dispatch<React.SetStateAction<string | undefined>>;
  setSettingsDetailPinned: React.Dispatch<React.SetStateAction<boolean>>;
  setSettingsReturnView: React.Dispatch<React.SetStateAction<SidebarView>>;
  setSettingsTab: React.Dispatch<React.SetStateAction<SettingsTab>>;
  settingsTab: SettingsTab;
  sourceMode: RecordingSourceMode;
  sourceReadiness: RecordingSourceReadinessDto | undefined;
  state: NotesState;
  takeNewTabIntent: () => boolean;
};
