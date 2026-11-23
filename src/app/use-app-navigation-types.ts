import type { SidebarView } from "../components/sidebar/Sidebar";
import type { Tab, TabNav } from "./tabs/tabs";
import type { NotesAction, NotesState } from "./state/app-state";
import type * as React from "react";

export type UseAppNavigationDependencies = {
  activeTabId: string;
  activeTabIdRef: React.MutableRefObject<string>;
  activeView: SidebarView;
  activeViewRef: React.MutableRefObject<SidebarView>;
  dispatch: React.Dispatch<NotesAction>;
  originAllNotes: boolean;
  originFolderId: string | undefined;
  restoreTargetRef: React.MutableRefObject<TabNav | null>;
  selectedNoteId: string | undefined;
  setActiveTabId: React.Dispatch<React.SetStateAction<string>>;
  setActiveView: React.Dispatch<React.SetStateAction<SidebarView>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setFolderReturnTarget: React.Dispatch<
    React.SetStateAction<{ noteId: string; label: string } | undefined>
  >;
  setOriginAllNotes: React.Dispatch<React.SetStateAction<boolean>>;
  setOriginFolderId: React.Dispatch<React.SetStateAction<string | undefined>>;
  setSettingsReturnView: React.Dispatch<React.SetStateAction<SidebarView>>;
  setTabs: React.Dispatch<React.SetStateAction<Tab[]>>;
  state: NotesState;
  tabs: Tab[];
  tabsRef: React.MutableRefObject<Tab[]>;
};
