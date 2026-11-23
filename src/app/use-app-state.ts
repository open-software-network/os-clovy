import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import type { NotesListHandle } from "../components/notes-list/NotesList";
import type { SettingsTab } from "../components/settings/AppSettings";
import type { SidebarView } from "../components/sidebar/Sidebar";
import { defaultNav, makeTabId, type Tab, type TabNav } from "./tabs/tabs";
import type { LiveTranscriptEventDto } from "../lib/tauri";
import { useCurrentDataPartitionName } from "../lib/data-partition";
import type { RecordingInactivityTracker } from "../lib/recording-inactivity";
import {
  createRecordingTelemetryStore,
  type RecordingTelemetryStore,
} from "../lib/recording-telemetry-store";
import type { NoteDto } from "../lib/tauri";
import type { RecordingSourceMode, RecordingSourceReadinessDto } from "../lib/tauri";
import { createInitialState, notesReducer } from "./state/app-state";
import { SIDEBAR_DEFAULT_WIDTH, type RecordingInactivityPrompt } from "./app-shell";

export function useAppState() {
  const currentDataPartitionName = useCurrentDataPartitionName();
  const [dataPartitionRefreshRevision, setDataPartitionRefreshRevision] = useState(0);
  const [state, dispatch] = useReducer(notesReducer, undefined, createInitialState);
  const [error, setError] = useState<string | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(SIDEBAR_DEFAULT_WIDTH);
  const [sidebarResizing, setSidebarResizing] = useState(false);
  const [sidebarTransition, setSidebarTransition] = useState<"none" | "smooth">("none");
  const [bootstrapped, setBootstrapped] = useState(false);
  const [activeView, setActiveView] = useState<SidebarView>("today");
  const activeViewRef = useRef(activeView);
  activeViewRef.current = activeView;
  const [activeTabId, setActiveTabId] = useState(makeTabId);
  const [tabs, setTabs] = useState<Tab[]>(() => [{ id: activeTabId, nav: defaultNav() }]);
  const tabsRef = useRef(tabs);
  const activeTabIdRef = useRef(activeTabId);
  // Set while restoring a tab's snapshot into live state: the capture effect
  // skips writes until live navigation settles onto the target (note loads are
  // async), so a half-applied snapshot never overwrites the tab it came from.
  const restoreTargetRef = useRef<TabNav | null>(null);
  const folderChangesRef = useRef({
    notes: new Map<string, number>(),
    deletedProjects: new Map<string, number>(),
  });
  const mainPanelBodyRef = useRef<HTMLDivElement | null>(null);
  const noteDetailScrollRef = useRef<HTMLDivElement | null>(null);
  const notesListRef = useRef<NotesListHandle | null>(null);
  // Where the back affordance in settings returns to — captured when settings
  // is opened so "back" lands the user where they were, not on Notes.
  const [settingsReturnView, setSettingsReturnView] = useState<SidebarView>("today");
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("general");
  // When the Memory tab is opened from a project ("Manage memories"), the
  // manager pre-filters to that project. Cleared on any normal tab navigation.
  const [memoryFolderFilter, setMemoryFolderFilter] = useState<string | undefined>();
  const openSettings = useCallback((tab?: SettingsTab) => {
    const returnView = activeViewRef.current;
    if (returnView !== "settings") {
      setSettingsReturnView(returnView);
    }
    setMemoryFolderFilter(undefined);
    if (tab) setSettingsTab(tab);
    setActiveView("settings");
  }, []);
  // Deep-link into Settings > Memory filtered to a project (from the project
  // settings dialog's "Manage memories").
  const openMemorySettings = useCallback((folderId?: string) => {
    const returnView = activeViewRef.current;
    if (returnView !== "settings") {
      setSettingsReturnView(returnView);
    }
    setMemoryFolderFilter(folderId);
    setSettingsTab("memory");
    setActiveView("settings");
  }, []);
  // Any deliberate tab change clears the project pre-filter so opening Memory
  // from the settings nav shows all memories, not a stale project scope.
  const changeSettingsTab = useCallback((tab: SettingsTab) => {
    setMemoryFolderFilter(undefined);
    setSettingsTab(tab);
  }, []);
  const [originFolderId, setOriginFolderId] = useState<string | undefined>();
  // Tracks that the open note was drilled into from the All notes view, so the
  // note shows the same back-arrow + breadcrumb chrome folders use. Cleared
  // whenever a note is opened from anywhere else (e.g. the sidebar list).
  const [originAllNotes, setOriginAllNotes] = useState(false);
  const [folderReturnTarget, setFolderReturnTarget] = useState<
    { noteId: string; label: string } | undefined
  >();
  const [moveDialogNoteIds, setMoveDialogNoteIds] = useState<string[] | null>(null);
  // User's intent for system audio. Defaults true ("record everything").
  // The actual sourceMode is derived below so that granting/revoking
  // permission in System Settings flips the toggle without losing intent.
  const [userWantsSystemAudio, setUserWantsSystemAudio] = useState(true);
  const [sourceReadiness, setSourceReadiness] = useState<RecordingSourceReadinessDto>();
  const [checkingSourceReadiness, setCheckingSourceReadiness] = useState(false);
  const [accessibilityStatus, setAccessibilityStatus] = useState<string>();
  const [accessibilityBannerDismissed, setAccessibilityBannerDismissed] = useState(false);
  const [systemAudioRefreshRequest, setSystemAudioRefreshRequest] = useState(0);
  const [microphoneStatus, setMicrophoneStatus] = useState<string>();
  // `ready` only says this device is capable of system capture; the platform
  // grant/status is established by a microphone-plus-system probe.
  const systemSourceReadiness = sourceReadiness?.sources.find(
    (source) => source.source === "system",
  );
  const systemGranted =
    systemSourceReadiness?.ready === true && systemSourceReadiness.permissionState === "granted";
  const recordingState = state.recordingStatus?.state;
  const captureActive =
    recordingState === "recording" ||
    recordingState === "paused" ||
    recordingState === "finalizing" ||
    recordingState === "validating";
  const sourceMode: RecordingSourceMode =
    userWantsSystemAudio && systemGranted ? "microphonePlusSystem" : "microphoneOnly";
  // The note the active recording session belongs to. recordingStatus carries
  // no noteId, so without this the finish flow could only guess from the
  // currently selected note — wrong whenever the user browsed away while
  // recording.
  const recordingNoteIdRef = useRef<string | undefined>(undefined);
  // A recording may deliberately keep its owning note visible after the user
  // switches data partitions. Remember that exception so stopping the take can
  // remove the previous partition's note and tab snapshots immediately.
  const crossPartitionRecordingNoteIdRef = useRef<string | undefined>(undefined);
  // Calendar matching finishes in the background and emits a global event.
  // Remember which partition started each lookup so a late result cannot
  // upsert an old note into whichever partition is visible when it arrives.
  const calendarContextNotePartitionsRef = useRef(new Map<string, string>());
  const calendarContextNoteUpdatesRef = useRef(new Map<string, NoteDto>());
  const pendingCalendarContextAdoptionsRef = useRef(new Set<string>());
  // Reactive mirror of recordingNoteIdRef. The ref serves the async finish/HUD
  // paths that need the latest value synchronously; this state drives render
  // decisions — which note shows the in-note RecorderBar, and whether the
  // floating global recorder pill is up (it shows whenever a recording is live
  // but you're not viewing its note). Always update them together via
  // setRecordingNote so they can't drift.
  const [recordingNoteId, setRecordingNoteIdState] = useState<string | undefined>(undefined);
  const recordingStatusRef = useRef(state.recordingStatus);
  const recordingTelemetryStoreRef = useRef<RecordingTelemetryStore | null>(null);
  if (!recordingTelemetryStoreRef.current) {
    recordingTelemetryStoreRef.current = createRecordingTelemetryStore(state.recordingStatus);
  }
  const recordingTelemetryStore = recordingTelemetryStoreRef.current;
  const dictationWorkflowActiveRef = useRef(false);
  const recordingInactivityTrackerRef = useRef<RecordingInactivityTracker>({});
  const [recordingInactivityPrompt, setRecordingInactivityPrompt] =
    useState<RecordingInactivityPrompt | null>(null);
  const [recordingInactivityNow, setRecordingInactivityNow] = useState(() => Date.now());
  const recordingStartInFlightRef = useRef(false);
  const [liveTranscriptEvents, setLiveTranscriptEvents] = useState<LiveTranscriptEventDto[]>([]);
  useEffect(() => {
    recordingStatusRef.current = state.recordingStatus;
    recordingTelemetryStore.setStatus(state.recordingStatus);
  }, [recordingTelemetryStore, state.recordingStatus]);
  const setRecordingNote = useCallback((noteId: string | undefined) => {
    recordingNoteIdRef.current = noteId;
    setRecordingNoteIdState(noteId);
  }, []);

  return {
    currentDataPartitionName,
    dataPartitionRefreshRevision,
    setDataPartitionRefreshRevision,
    state,
    dispatch,
    error,
    setError,
    sidebarCollapsed,
    setSidebarCollapsed,
    sidebarWidth,
    setSidebarWidth,
    sidebarResizing,
    setSidebarResizing,
    sidebarTransition,
    setSidebarTransition,
    bootstrapped,
    setBootstrapped,
    activeView,
    setActiveView,
    activeViewRef,
    tabs,
    setTabs,
    activeTabId,
    setActiveTabId,
    tabsRef,
    activeTabIdRef,
    restoreTargetRef,
    folderChangesRef,
    mainPanelBodyRef,
    noteDetailScrollRef,
    notesListRef,
    settingsReturnView,
    setSettingsReturnView,
    settingsTab,
    setSettingsTab,
    memoryFolderFilter,
    openSettings,
    openMemorySettings,
    changeSettingsTab,
    originFolderId,
    setOriginFolderId,
    originAllNotes,
    setOriginAllNotes,
    folderReturnTarget,
    setFolderReturnTarget,
    moveDialogNoteIds,
    setMoveDialogNoteIds,
    setUserWantsSystemAudio,
    sourceReadiness,
    setSourceReadiness,
    checkingSourceReadiness,
    setCheckingSourceReadiness,
    accessibilityStatus,
    setAccessibilityStatus,
    accessibilityBannerDismissed,
    setAccessibilityBannerDismissed,
    systemAudioRefreshRequest,
    setSystemAudioRefreshRequest,
    microphoneStatus,
    setMicrophoneStatus,
    systemGranted,
    captureActive,
    sourceMode,
    recordingNoteIdRef,
    crossPartitionRecordingNoteIdRef,
    calendarContextNotePartitionsRef,
    calendarContextNoteUpdatesRef,
    pendingCalendarContextAdoptionsRef,
    recordingNoteId,
    recordingTelemetryStore,
    recordingStatusRef,
    dictationWorkflowActiveRef,
    recordingInactivityTrackerRef,
    recordingInactivityPrompt,
    setRecordingInactivityPrompt,
    recordingInactivityNow,
    setRecordingInactivityNow,
    recordingStartInFlightRef,
    liveTranscriptEvents,
    setLiveTranscriptEvents,
    setRecordingNote,
  };
}
