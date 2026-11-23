import type { SidebarView } from "../components/sidebar/Sidebar";
import type { Tab } from "./tabs/tabs";
import type { NotesAction } from "./state/app-state";
import type * as React from "react";

export type UseDataPartitionRefreshDependencies = {
  currentDataPartitionName: string;
  activeViewRef: React.MutableRefObject<SidebarView>;
  appBlocked: boolean;
  bootstrapped: boolean;
  crossPartitionRecordingNoteIdRef: React.MutableRefObject<string | undefined>;
  dispatch: React.Dispatch<NotesAction>;
  lastDataPartitionRef: React.MutableRefObject<string | undefined>;
  lastDataPartitionRefreshRevisionRef: React.MutableRefObject<number>;
  dataPartitionRefreshRevision: number;
  recordingNoteIdRef: React.MutableRefObject<string | undefined>;
  setActiveView: React.Dispatch<React.SetStateAction<SidebarView>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setFolderReturnTarget: React.Dispatch<
    React.SetStateAction<{ noteId: string; label: string } | undefined>
  >;
  setOriginAllNotes: React.Dispatch<React.SetStateAction<boolean>>;
  setOriginFolderId: React.Dispatch<React.SetStateAction<string | undefined>>;
  setTabs: React.Dispatch<React.SetStateAction<Tab[]>>;
  tabsRef: React.MutableRefObject<Tab[]>;
};
