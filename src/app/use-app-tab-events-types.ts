import type { Tab } from "./tabs/tabs";
import type { NotesAction } from "./state/app-state";
import type { NoteDto } from "../lib/tauri";
import type * as React from "react";

export type UseAppTabEventsDependencies = {
  activateTab: (id: string) => void;
  activeTabId: string;
  activeTabIdRef: React.MutableRefObject<string>;
  calendarContextNotePartitionsRef: React.MutableRefObject<Map<string, string>>;
  calendarContextNoteUpdatesRef: React.MutableRefObject<Map<string, NoteDto>>;
  closeTab: (id: string) => void;
  cycleTab: (delta: number) => void;
  dispatch: React.Dispatch<NotesAction>;
  openNewTab: () => void;
  pendingCalendarContextAdoptionsRef: React.MutableRefObject<Set<string>>;
  setTabs: React.Dispatch<React.SetStateAction<Tab[]>>;
  tabs: Tab[];
};
