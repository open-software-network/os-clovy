import type { NotesListHandle } from "../components/notes-list/NotesList";
import type { SettingsTab } from "../components/settings/AppSettings";
import type { SidebarView } from "../components/sidebar/Sidebar";
import type { TabItem } from "../components/tabs/TabBar";
import type { TabNav } from "./tabs/tabs";
import type { FolderDto, NoteDto, RecordingStatusDto } from "../lib/tauri";
import type { RecordingInactivityPrompt } from "./app-shell";
import type { NotesAction, NotesState } from "./state/app-state";
import type { UpdateHubProps } from "./app-effects/update-ui";
import type * as React from "react";

export type RenderAppLayoutDependencies = {
  updateHub: UpdateHubProps;
  accessibilityBannerDismissed: boolean;
  accessibilityBlocked: boolean;
  activateTab: (id: string) => void;
  activeTabId: string;
  activeView: SidebarView;
  changeSettingsTab: (tab: SettingsTab) => void;
  closeOtherTabs: (id: string) => void;
  closeTab: (id: string) => void;
  confirmDeleteNote: boolean;
  detailScrollerActive: boolean;
  dispatch: React.Dispatch<NotesAction>;
  error: string | null;
  handleCreateFolder: (name: string, description?: string) => Promise<FolderDto | undefined>;
  handleDeleteNote: (noteId: string) => Promise<void>;
  handleEnableAccessibility: () => void;
  handleKeepRecordingAfterInactivityPrompt: () => void;
  handleOpenRecordingNote: () => Promise<void>;
  handlePauseRecordingAfterInactivityPrompt: () => void;
  handleRemoveNoteFromFolder: (
    noteId: string,
    folderId: string,
    options?: { rethrow?: boolean },
  ) => Promise<void>;
  handleReorderTabs: (orderedVisibleIds: string[]) => void;
  handleSelectNote: (noteId: string) => Promise<void>;
  handleSetNoteFolder: (
    noteId: string,
    folderId: string,
    options?: { rethrow?: boolean },
  ) => Promise<void>;
  mainPanelBodyRef: React.MutableRefObject<HTMLDivElement | null>;
  moveDialogNoteIds: string[] | null;
  noteDetailScrollerActive: boolean;
  notesListRef: React.MutableRefObject<NotesListHandle | null>;
  openSettings: (tab?: SettingsTab) => void;
  openNewTab: () => void;
  openTab: (nav: TabNav) => void;
  pillIsDemo: boolean;
  pillStatus: RecordingStatusDto | null;
  recordingInactivityPrompt: RecordingInactivityPrompt | null;
  recordingInactivitySecondsRemaining: number;
  recordingNoteTitle: string;
  selectedNote: NoteDto | undefined;
  setAccessibilityBannerDismissed: React.Dispatch<React.SetStateAction<boolean>>;
  setActiveView: React.Dispatch<React.SetStateAction<SidebarView>>;
  setConfirmDeleteNote: React.Dispatch<React.SetStateAction<boolean>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setFolderReturnTarget: React.Dispatch<
    React.SetStateAction<{ noteId: string; label: string } | undefined>
  >;
  setMoveDialogNoteIds: React.Dispatch<React.SetStateAction<string[] | null>>;
  setOriginAllNotes: React.Dispatch<React.SetStateAction<boolean>>;
  setOriginFolderId: React.Dispatch<React.SetStateAction<string | undefined>>;
  setSidebarCollapsed: React.Dispatch<React.SetStateAction<boolean>>;
  setSidebarResizing: React.Dispatch<React.SetStateAction<boolean>>;
  setSidebarTransition: React.Dispatch<React.SetStateAction<"none" | "smooth">>;
  setSidebarWidth: React.Dispatch<React.SetStateAction<number>>;
  settingsDetailScrollerActive: boolean;
  settingsReturnView: SidebarView;
  settingsTab: SettingsTab;
  sidebarCollapsed: boolean;
  sidebarRecorderStatus: RecordingStatusDto | null;
  sidebarResizing: boolean;
  sidebarTransition: "none" | "smooth";
  sidebarWidth: number;
  state: NotesState;
  tabItems: TabItem[];
  takeNewTabIntent: () => boolean;
  workspaceContent: JSX.Element;
};
