import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { NoteHeaderActions } from "../components/note-editor/NoteHeaderActions";
import { toast } from "../components/ui/Toaster";
import { t, useLocale } from "../i18n";
import { exportNoteAsPdf } from "../lib/note-pdf";
import { SETTINGS_TABS } from "../components/settings/AppSettings";
import type { TabItem } from "../components/tabs/TabBar";
import { reorderTabs } from "./tabs/tabs";
import {
  checkRecordingSourceReadiness,
  createFolder,
  createNote,
  dictationHelperCommand,
  downloadNoteAudio,
  getNote,
  keepMeetingRecording,
  LIVE_TRANSCRIPT_EVENT,
  openPrivacySettings,
  recoverRecording,
  revealPath,
  renameFolder,
  completeNoteSaveFlush,
  NOTE_SAVE_FLUSH_REQUESTED_EVENT,
  patchNote,
  queueMeetingEndFinishRequest,
  type LiveTranscriptEventDto,
  type MeetingEndStatus,
  type NoteDto,
} from "../lib/tauri";
import { preloadRecordingSounds } from "../lib/recording-sounds";
import { preloadAgentSounds } from "../lib/agent-sounds";
import { messageFromError } from "../lib/errors";
import {
  getCurrentDataPartitionName,
  DATA_PARTITION_CHANGED_EVENT,
  type DataPartitionChangedDetail,
} from "../lib/data-partition";
import {
  authoritativeTranscriptCoverageKey,
  clearTerminalLiveTranscriptEvents,
  upsertLiveTranscriptEvent,
} from "../lib/live-transcript-preview";
import {
  RECORDING_INACTIVITY_RESPONSE_MS,
  RECORDING_INACTIVITY_SNOOZE_MS,
  nextRecordingInactivityDecision,
  recordingHasActivity,
} from "../lib/recording-inactivity";
import {
  notifyRecordingAutoPaused,
  notifyRecordingStillMeetingPrompt,
} from "../lib/recording-notifications";
import { RecordingTelemetryProvider } from "../lib/recording-telemetry-store";
import { OPEN_SETTINGS_EVENT } from "../lib/menu-bar";
import type { FolderDto } from "../lib/tauri";
import type { RecordingSourceMode } from "../lib/tauri";
import { retryPendingAutostartDefault } from "../lib/autostart";
import { applyOnboardingReplayFlag, isOnboardingComplete } from "../lib/onboarding";
import { attachScrollThumbFade } from "../lib/scroll-thumb-fade";
import {
  isAccessibilityBlocked,
  isCreateNoteShortcut,
  isMicrophoneRecordingBlocked,
} from "./app-helpers";
import {
  ACCESSIBILITY_PERMISSION_REFRESH_INTERVAL_MS,
  SYSTEM_AUDIO_PERMISSION_REFRESH_INTERVAL_MS,
  SYSTEM_AUDIO_PERMISSION_REFRESH_TIMEOUT_MS,
  noteHasDownloadableAudio,
  tabMeta,
} from "./app-shell";
export { isAccessibilityBlocked, isMicrophoneRecordingBlocked } from "./app-helpers";

import { useAppExternalEvents } from "./use-app-external-events";

import { useAgentAttentionNotifications } from "./use-agent-attention-notifications";
import { startAgentRunSettlementRelay } from "../lib/agent-run-settlement";

import { useDataPartitionRefresh } from "./use-data-partition-refresh";

import { useRecordingStartActions } from "./use-recording-start-actions";

import { useRecordingEvents } from "./use-recording-events";
import { useMeetingEndEvents } from "./use-meeting-end-events";

import { useRecordingControls } from "./use-recording-controls";

import { useAppNavigation } from "./use-app-navigation";

import { renderAppWorkspace } from "./app-workspace-view";

import { renderAppLayout } from "./app-layout";

import { createAppDomainActions } from "./app-domain-actions";

import { useAppUpdateActions } from "./use-app-update-actions";

import { createNoteActions } from "./note-actions";
import { NoteSaveController } from "./note-save-controller";

import { useDictationEvents } from "./use-dictation-events";

import { useAppBootstrap } from "./use-app-bootstrap";

import { useAppTabEvents } from "./use-app-tab-events";

import { useAppDevDemos } from "./use-app-dev-demos";

import { useRecordingTelemetry } from "./use-recording-telemetry";

import { useNoteProcessingEvents } from "./use-note-processing-events";

import { useAppState } from "./use-app-state";
import { renderAppOnboarding } from "./app-onboarding";

export function App() {
  // Subscribes the shell to the interface language: the render helpers and
  // tab titles below read `t` at render time.
  const locale = useLocale();
  const {
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
  } = useAppState();
  const [meetingEndStatus, setMeetingEndStatus] = useState<MeetingEndStatus | null>(null);
  const [meetingEndNow, setMeetingEndNow] = useState(() => Date.now());
  const meetingEndReadyRef = useRef(false);
  const meetingEndListenerRegisteredRef = useRef(false);
  const drainPendingMeetingEndFinishRef = useRef<() => void>(() => {});
  const meetingEndFinishHandlerRef = useRef<(sessionId: string) => Promise<boolean>>(
    async () => false,
  );
  const noteSaveControllerRef = useRef<NoteSaveController | null>(null);
  if (!noteSaveControllerRef.current) {
    noteSaveControllerRef.current = new NoteSaveController({
      persist: patchNote,
      onPersisted: (patch) => {
        dispatch({ type: "notePatched", noteId: patch.id, patch });
      },
      onError: (saveError) => {
        setError(messageFromError(saveError));
      },
    });
  }
  const noteSaveController = noteSaveControllerRef.current;

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listen<{ requestId: string }>(NOTE_SAVE_FLUSH_REQUESTED_EVENT, async (event) => {
      try {
        await noteSaveController.flushAll();
        await completeNoteSaveFlush(event.payload.requestId);
      } catch (saveError) {
        setError(messageFromError(saveError));
      }
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      disposed = true;
      unlisten?.();
      void noteSaveController.flushAll().catch(() => {
        // Native shutdown owns the durable barrier. React cleanup is only a
        // best-effort backstop, and persist() has already surfaced the error.
      });
    };
  }, [noteSaveController, setError]);

  function getSelectedNoteId() {
    return selectedNoteIdRef.current;
  }
  // Dev-only synthetic status driving the global recorder pill, set by the
  // window.__globalRecorderPill console hook. When non-null it force-shows the
  // pill (any view, no real recording) so its styling can be inspected.
  const {
    demoRecorderStatus,
    recordNoticesConsentPinned,
    recordNoticesMicOverride,
    recordNoticesDemoRef,
    finishingSessionsRef,
  } = useAppDevDemos({
    dispatch,
    getSelectedNoteId,
    recordingStatusRef,
    setActiveView,
    setLiveTranscriptEvents,
    setRecordingNote,
  });
  const [onboardingDone, setOnboardingDone] = useState(() => {
    applyOnboardingReplayFlag();
    return isOnboardingComplete();
  });
  // The wizard handles personalization and permissions.
  const onboardingRequired = !onboardingDone;
  // Onboarding holds bootstrap and eager permission probes because the
  // wizard owns the permission prompts while it is on screen.
  const appBlocked = onboardingRequired;
  meetingEndReadyRef.current = !appBlocked && bootstrapped;
  const selectedNote = state.selectedNote;
  const selectedNoteId = selectedNote?.id;
  const visibleEditorNoteId = activeView === "meetings" ? selectedNoteId : undefined;
  const previousVisibleEditorNoteIdRef = useRef<string | undefined>(undefined);
  const latestNoteRequestRef = useRef(0);
  useEffect(() => {
    const previousNoteId = previousVisibleEditorNoteIdRef.current;
    previousVisibleEditorNoteIdRef.current = visibleEditorNoteId;
    if (previousNoteId && previousNoteId !== visibleEditorNoteId) {
      void noteSaveController.flush(previousNoteId).catch((saveError) => {
        setError(messageFromError(saveError));
      });
    }
  }, [noteSaveController, setError, visibleEditorNoteId]);
  const selectedNoteLiveTranscript = useMemo(
    () => liveTranscriptEvents.filter((event) => event.noteId === selectedNoteId),
    [liveTranscriptEvents, selectedNoteId],
  );
  const selectedNoteTranscriptCoverageKey = authoritativeTranscriptCoverageKey(
    selectedNote?.sourceTranscripts ?? [],
  );
  const [confirmDeleteNote, setConfirmDeleteNote] = useState(false);
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset note-scoped UI on selection change
  useEffect(() => {
    setConfirmDeleteNote(false);
  }, [selectedNoteId]);
  async function handleExportNotePdf() {
    if (!selectedNote) return;
    await exportNoteAsPdf(selectedNote.title, {
      showNotes:
        selectedNote.activeTab === "transcription"
          ? async () => {
              await handleSaveNoteNow(selectedNote.id, { activeTab: "notes" });
            }
          : undefined,
    });
  }
  async function handleDownloadNoteAudio() {
    if (!selectedNote) return;
    try {
      const result = await downloadNoteAudio(selectedNote.id);
      toast.success(t("app.audio.downloaded"), {
        action: {
          label: t("app.audio.showFile"),
          onClick: () => {
            void revealPath(result.path).catch((err: unknown) => {
              toast.error(messageFromError(err));
            });
          },
        },
      });
    } catch (err) {
      toast.error(messageFromError(err));
    }
  }
  const noteToolbarActions = selectedNote ? (
    <NoteHeaderActions
      noteId={selectedNote.id}
      noteTitle={selectedNote.title}
      onExportPdf={() => void handleExportNotePdf()}
      onDownloadAudio={
        noteHasDownloadableAudio(selectedNote) ? () => void handleDownloadNoteAudio() : undefined
      }
      onDelete={() => setConfirmDeleteNote(true)}
    />
  ) : null;
  const originFolder = originFolderId
    ? state.folders.find((folder) => folder.id === originFolderId)
    : undefined;
  const recoveriesByNote = useMemo(() => {
    const map = new Map<string, (typeof state.activeRecoveries)[number]>();
    for (const recovery of state.activeRecoveries) {
      // If multiple recoveries land on one note, the first one wins —
      // backend should only surface one per note in practice.
      if (!map.has(recovery.noteId)) map.set(recovery.noteId, recovery);
    }
    return map;
  }, [state.activeRecoveries]);
  const selectedRecovery = selectedNote ? recoveriesByNote.get(selectedNote.id) : undefined;
  const noteDetailScrollerActive = activeView === "meetings" && !!selectedNote;
  const detailScrollerActive = activeView === "folders" && !!state.selectedFolderId;
  // A settings drill-in (e.g. a skill detail) that pins its own frosted
  // breadcrumb bar at the top of the panel and scrolls its content beneath —
  // the same pinned-bar mechanic as opening a meeting note from a folder. When
  // set, the outer body stops scrolling so the pinned bar stays fixed.
  const [settingsDetailPinned, setSettingsDetailPinned] = useState(false);
  const settingsDetailScrollerActive = activeView === "settings" && settingsDetailPinned;

  // ---- Tabs ------------------------------------------------------------
  // The current live navigation, reduced to a snapshot. Fields are gated by
  // view so the active tab only churns when something it actually shows
  // changes (see navEquals).
  const { applyNav, activateTab, openTab, openNewTab, closeTab, selectedNoteIdRef } =
    useAppNavigation({
      activeTabId,
      activeTabIdRef,
      activeView,
      activeViewRef,
      dispatch,
      originAllNotes,
      originFolderId,
      restoreTargetRef,
      selectedNoteId,
      setActiveTabId,
      setActiveView,
      setError,
      setFolderReturnTarget,
      setOriginAllNotes,
      setOriginFolderId,
      setSettingsReturnView,
      setTabs,
      state,
      tabs,
      tabsRef,
    });

  // Keep only the given tab, focusing it. From the tab right-click menu.
  function closeOtherTabs(id: string) {
    const keep = tabs.find((tab) => tab.id === id);
    if (!keep || tabs.length <= 1) return;
    setTabs([keep]);
    if (id !== activeTabId) {
      setActiveTabId(id);
      applyNav(keep.nav);
    }
  }

  // Drag-reorder from the tab strip: the visible tabs land in their new order,
  // overflow tabs stay put (see reorderTabs).
  function handleReorderTabs(orderedVisibleIds: string[]) {
    setTabs((prev) => reorderTabs(prev, orderedVisibleIds));
  }

  function cycleTab(delta: number) {
    if (tabs.length <= 1) return;
    const index = tabs.findIndex((tab) => tab.id === activeTabId);
    if (index < 0) return;
    const target = tabs[(index + delta + tabs.length) % tabs.length];
    if (target) activateTab(target.id);
  }

  // Drop tabs whose note was deleted. The active tab is kept regardless: the
  // delete handlers already move its selection on to the next note, and the
  // capture effect reconciles its snapshot — pruning it here would fight that.
  const { pruneDeletedNoteTabs, takeNewTabIntent } = useAppTabEvents({
    activateTab,
    activeTabId,
    activeTabIdRef,
    calendarContextNotePartitionsRef,
    calendarContextNoteUpdatesRef,
    closeTab,
    cycleTab,
    dispatch,
    openNewTab,
    pendingCalendarContextAdoptionsRef,
    setTabs,
    tabs,
  });

  // The label of the active settings section, so a tab parked on the Settings
  // view reads e.g. "MCP servers" instead of the generic "Settings". Only the
  // active tab is on the live settings view, so the section label applies to it.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `locale` re-derives translated labels when the interface language changes.
  const settingsSectionLabel = useMemo(
    () => SETTINGS_TABS.find((tab) => tab.id === settingsTab)?.label,
    [settingsTab, locale],
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: `locale` re-derives translated labels when the interface language changes.
  const tabItems = useMemo<TabItem[]>(
    () =>
      tabs.map((tab) => ({
        id: tab.id,
        ...tabMeta(
          tab.nav,
          state.notes,
          state.folders,
          tab.id === activeTabId ? settingsSectionLabel : undefined,
        ),
      })),
    [tabs, state.notes, state.folders, activeTabId, settingsSectionLabel, locale],
  );

  function handleRecovery(sessionId: string, action: "validate" | "discard") {
    const recoveryNoteId = state.activeRecoveries.find(
      (recovery) => recovery.sessionId === sessionId,
    )?.noteId;
    void (async () => {
      try {
        const note = await recoverRecording(sessionId, action);
        clearActiveRecordingSession(sessionId);
        dispatch({ type: "noteProcessingUpdated", note });
        dispatch({ type: "recoveryRemoved", sessionId });
      } catch (err) {
        if (
          action === "validate" &&
          recoveryNoteId &&
          (await applyNoteScopedProcessingFailure(recoveryNoteId, err))
        ) {
          clearActiveRecordingSession(sessionId);
          dispatch({ type: "recoveryRemoved", sessionId });
          return;
        }
        setError(messageFromError(err));
      }
    })();
  }

  function clearActiveRecordingSession(sessionId: string) {
    if (recordingStatusRef.current?.sessionId !== sessionId) return;
    recordingStatusRef.current = undefined;
    setRecordingNote(undefined);
    dispatch({ type: "recordingStatusCleared" });
  }

  useEffect(() => {
    preloadRecordingSounds();
    preloadAgentSounds();
  }, []);

  // The card scroller's thumb fades in with scroll/pointer activity and back
  // out when idle (native-overlay feel; see scroll-thumb-fade.ts).
  useEffect(() => {
    const el = mainPanelBodyRef.current;
    if (!el) return;
    return attachScrollThumbFade(el);
  }, []);

  useEffect(() => {
    if (!noteDetailScrollerActive) return;
    const el = noteDetailScrollRef.current;
    if (!el) return;
    return attachScrollThumbFade(el);
  }, [noteDetailScrollerActive, selectedNoteId]);

  // Leaving the settings view drops any pinned settings drill-in, so the body
  // scroller is never left frozen for a view that has no pinned bar.
  useEffect(() => {
    if (activeView !== "settings") setSettingsDetailPinned(false);
  }, [activeView]);

  useEffect(() => {
    let aborted = false;
    let unlisten: (() => void) | undefined;
    void listen(OPEN_SETTINGS_EVENT, () => openSettings()).then((cleanup) => {
      if (aborted) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      aborted = true;
      unlisten?.();
    };
  }, [openSettings]);

  useAppExternalEvents(setActiveView);

  useEffect(
    () =>
      startAgentRunSettlementRelay({
        resolveTitle: () => undefined,
        readQueuedFollowUps: () => ({}),
      }),
    [],
  );
  useAgentAttentionNotifications({ activeViewRef, dictationWorkflowActiveRef, recordingStatusRef });

  const flushNotesForUpdate = useCallback(
    () => noteSaveController.flushAll(),
    [noteSaveController],
  );
  const { updatesEnabled, runUpdateCheck, checkingForUpdates, updateHub } = useAppUpdateActions({
    appBlocked,
    recordingActive: captureActive,
    dictationWorkflowActiveRef,
    flushNotes: flushNotesForUpdate,
  });

  // A fresh install whose automatic launch-at-login enable failed during
  // onboarding completion gets another chance on every normal startup:
  // completion hides the wizard, so without this the retry would wait for
  // an onboarding version bump. No-ops once the default has been settled
  // (applied, or overridden by an explicit Settings toggle).
  useEffect(() => {
    if (appBlocked) return;
    void retryPendingAutostartDefault();
  }, [appBlocked]);

  useDictationEvents({
    dictationWorkflowActiveRef,
    setAccessibilityStatus,
    setActiveView,
    setMicrophoneStatus,
  });

  // The detached meeting HUD (shown when Clovy is backgrounded, minimized, or
  // hidden mid-recording) is a presence indicator, not a control surface:
  // clicking it emits "reopen", and we bring the window forward and land back
  // on the meeting being recorded. All recording controls stay in-app.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let aborted = false;
    void listen<{ action: "reopen"; noteId?: string }>("meeting-hud-action", (event) => {
      if (event.payload?.action !== "reopen") return;
      const main = getCurrentWindow();
      void main.show();
      void main.unminimize();
      void main.setFocus();
      const noteId = event.payload.noteId ?? recordingNoteIdRef.current;
      if (noteId) {
        setActiveView("meetings");
        void handleSelectNote(noteId);
      }
    }).then((cleanup) => {
      // If the listener resolves after unmount, tear it down immediately.
      if (aborted) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      aborted = true;
      unlisten?.();
    };
  }, []);

  const accessibilityBlocked = isAccessibilityBlocked(accessibilityStatus);
  useEffect(() => {
    if (!accessibilityBlocked) setAccessibilityBannerDismissed(false);
  }, [accessibilityBlocked]);

  // recordNoticesMicOverride is the dev __recordNoticesDemo hook parking the
  // mic-blocked notice; it is always null in production (the state never leaves
  // its initial value), so real behavior is untouched.
  const microphoneBlocked =
    recordNoticesMicOverride ?? isMicrophoneRecordingBlocked(microphoneStatus, sourceReadiness);

  const refreshPermissionStatuses = useCallback(() => {
    void dictationHelperCommand({ type: "get_permission_status" }).catch(() => undefined);
  }, []);

  useAppBootstrap({
    appBlocked,
    calendarContextNotePartitionsRef,
    calendarContextNoteUpdatesRef,
    dispatch,
    pendingCalendarContextAdoptionsRef,
    recordingStatusRef,
    setActiveView,
    setBootstrapped,
    setError,
    setRecordingNote,
  });

  useEffect(() => {
    function handleDataPartitionChanged(event: Event) {
      const detail = (event as CustomEvent<DataPartitionChangedDetail>).detail;
      if (!detail || detail.partition !== getCurrentDataPartitionName()) return;
      setDataPartitionRefreshRevision((revision) => revision + 1);
    }

    window.addEventListener(DATA_PARTITION_CHANGED_EVENT, handleDataPartitionChanged);
    return () => {
      window.removeEventListener(DATA_PARTITION_CHANGED_EVENT, handleDataPartitionChanged);
    };
  }, []);

  // A data partition switch swaps all visible data (ADR 0031): re-read scoped
  // notes and projects together. The same refresh
  // runs when data moves into the current partition, where its name does not
  // change. If a recording is running its note keeps the selection (get_note
  // is unscoped) so the recording view is never yanked mid-take.
  const lastDataPartitionRef = useRef<string | undefined>(undefined);
  const lastDataPartitionRefreshRevisionRef = useRef(0);
  useDataPartitionRefresh({
    currentDataPartitionName,
    activeViewRef,
    appBlocked,
    bootstrapped,
    crossPartitionRecordingNoteIdRef,
    dispatch,
    lastDataPartitionRef,
    lastDataPartitionRefreshRevisionRef,
    dataPartitionRefreshRevision,
    recordingNoteIdRef,
    setActiveView,
    setError,
    setFolderReturnTarget,
    setOriginAllNotes,
    setOriginFolderId,
    setTabs,
    tabsRef,
  });

  // Probe with "microphonePlusSystem" on mount so sourceReadiness always
  // has the system source. Onboarding's permissions screen normally fires
  // the native TCC prompt in context; for users who skipped that step the
  // helper preflight behind this call surfaces it here instead.
  useEffect(() => {
    if (appBlocked) return;
    let cancelled = false;
    setCheckingSourceReadiness(true);
    checkRecordingSourceReadiness("microphonePlusSystem")
      .then((readiness) => {
        if (!cancelled) setSourceReadiness(readiness);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(messageFromError(err));
      })
      .finally(() => {
        if (!cancelled) setCheckingSourceReadiness(false);
      });
    // Eagerly request mic from the helper. This fires the native TCC
    // prompt for fresh installs (matching the system-audio eager prompt),
    // and for already-denied users it immediately emits the current
    // status so the mic-blocked strip renders without further user
    // action. For granted users it's a no-op.
    void dictationHelperCommand({
      type: "request_microphone_permission",
    }).catch(() => undefined);
    // Check Accessibility on every app open. The helper grant is what lets
    // dictation paste into other apps; without this poll a fresh install
    // never learns the helper is untrusted (the focus refresh below doesn't
    // fire at launch), so the paste-permission banner would stay hidden.
    refreshPermissionStatuses();
    return () => {
      cancelled = true;
    };
  }, [appBlocked, refreshPermissionStatuses]);

  // Refresh permission state whenever the app regains focus — covers the
  // common case where the user flipped a toggle in System Settings and
  // returns to Clovy. The helper poll is what surfaces fresh mic /
  // accessibility state via the dictation-event listener above.
  useEffect(() => {
    if (appBlocked) return;
    function refresh() {
      refreshPermissionStatuses();
      if (captureActive) return;
      void checkRecordingSourceReadiness("microphonePlusSystem")
        .then(setSourceReadiness)
        .catch(() => undefined);
    }
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [appBlocked, captureActive, refreshPermissionStatuses]);

  // While Accessibility is missing, keep checking. macOS can leave stale
  // "June" rows from older builds in System Settings; after the current helper
  // is toggled, this notices the live TCC change without requiring a restart.
  useEffect(() => {
    if (appBlocked || !accessibilityBlocked) return;
    refreshPermissionStatuses();
    const interval = window.setInterval(
      refreshPermissionStatuses,
      ACCESSIBILITY_PERMISSION_REFRESH_INTERVAL_MS,
    );
    function onVisibilityChange() {
      if (document.visibilityState === "visible") refreshPermissionStatuses();
    }
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      window.clearInterval(interval);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [accessibilityBlocked, appBlocked, refreshPermissionStatuses]);

  // After the user opens System Settings for System Audio Recording, keep
  // checking briefly while macOS is in front. This matches Accessibility's
  // permission flow and avoids relying on a single webview focus event.
  useEffect(() => {
    if (appBlocked || captureActive || systemGranted || systemAudioRefreshRequest === 0) {
      return;
    }
    let cancelled = false;
    let inFlight = false;
    function poll() {
      if (inFlight) return;
      inFlight = true;
      void checkRecordingSourceReadiness("microphonePlusSystem")
        .then((readiness) => {
          if (!cancelled) setSourceReadiness(readiness);
        })
        .catch(() => undefined)
        .finally(() => {
          inFlight = false;
        });
    }
    poll();
    const interval = window.setInterval(poll, SYSTEM_AUDIO_PERMISSION_REFRESH_INTERVAL_MS);
    const timeout = window.setTimeout(() => {
      window.clearInterval(interval);
    }, SYSTEM_AUDIO_PERMISSION_REFRESH_TIMEOUT_MS);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      window.clearTimeout(timeout);
    };
  }, [appBlocked, captureActive, systemAudioRefreshRequest, systemGranted]);

  function handleSourceModeChange(next: RecordingSourceMode) {
    setUserWantsSystemAudio(next === "microphonePlusSystem");
  }

  // Explicit "Enable" action when system audio is denied. Sets intent on
  // (so the toggle auto-flips ON once permission is granted) and routes
  // the user to the System Settings pane.
  function handleEnableSystemAudio() {
    setUserWantsSystemAudio(true);
    setSystemAudioRefreshRequest((request) => request + 1);
    void openPrivacySettings("systemAudio");
  }

  function handleEnableMicrophone() {
    void openPrivacySettings("microphone");
  }

  function handleEnableAccessibility() {
    void dictationHelperCommand({
      type: "request_accessibility_permission",
    }).catch(async () => {
      try {
        await openPrivacySettings("accessibility");
      } catch {
        // The fallback is best-effort; there is no further recovery surface.
      }
    });
  }

  useRecordingTelemetry({
    dispatch,
    recordingTelemetryStore,
    recordingStatusRef,
  });

  useEffect(() => {
    if (
      !selectedNote ||
      (selectedNote.processingStatus !== "ready" && selectedNote.processingStatus !== "failed") ||
      (selectedNote.queuedRecordings ?? 0) > 0
    ) {
      return;
    }
    const protectedSessionIds = [...finishingSessionsRef.current];
    if (recordingNoteId === selectedNote.id && state.recordingStatus?.sessionId) {
      protectedSessionIds.push(state.recordingStatus.sessionId);
    }
    setLiveTranscriptEvents((current) =>
      clearTerminalLiveTranscriptEvents(
        current,
        selectedNote.id,
        selectedNote.sourceTranscripts ?? [],
        protectedSessionIds,
      ),
    );
  }, [
    recordingNoteId,
    selectedNote?.id,
    selectedNote?.processingStatus,
    selectedNote?.queuedRecordings,
    selectedNoteTranscriptCoverageKey,
    state.recordingStatus?.sessionId,
  ]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let aborted = false;
    void listen<LiveTranscriptEventDto>(LIVE_TRANSCRIPT_EVENT, (event) => {
      const payload = event.payload;
      const activeRecording = recordingStatusRef.current;
      if (!activeRecording || payload.sessionId !== activeRecording.sessionId) {
        return;
      }
      if (recordingNoteIdRef.current && payload.noteId !== recordingNoteIdRef.current) {
        return;
      }
      const text = payload.text.trim();
      if (!text) return;
      setLiveTranscriptEvents((current) =>
        upsertLiveTranscriptEvent(current, { ...payload, text }),
      );
    }).then((cleanup) => {
      if (aborted) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      aborted = true;
      unlisten?.();
    };
  }, []);

  useNoteProcessingEvents({
    dispatch,
    noteSaveController,
    folderChangesRef,
    notes: state.notes,
    selectedNote,
    liveTranscriptEvents,
    setError,
    setLiveTranscriptEvents,
  });

  const handleCreateNote = useCallback(
    async (folderId?: string | null) => {
      try {
        const targetFolderId = folderId === null ? undefined : (folderId ?? state.selectedFolderId);
        const note = await createNote(targetFolderId);
        dispatch({ type: "noteLoaded", note });
        setOriginFolderId(undefined);
        setOriginAllNotes(false);
        setActiveView("meetings");
      } catch (err) {
        setError(messageFromError(err));
      }
    },
    [state.selectedFolderId],
  );

  useEffect(() => {
    if (
      appBlocked ||
      !bootstrapped ||
      activeView !== "meetings" ||
      selectedNote ||
      state.selectedNoteId
    ) {
      return;
    }
    void handleCreateNote(null);
  }, [activeView, appBlocked, bootstrapped, handleCreateNote, selectedNote, state.selectedNoteId]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (document.querySelector('[role="dialog"]')) return;
      if (isCreateNoteShortcut(event)) {
        event.preventDefault();
        void handleCreateNote(null);
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [handleCreateNote]);

  function handleSelectFolder(folderId?: string) {
    setFolderReturnTarget(undefined);
    dispatch({ type: "folderSelected", folderId });
  }

  async function handleReturnToNote(noteId: string) {
    if (state.selectedNoteId !== noteId) {
      await handleSelectNote(noteId);
    }
    setActiveView("meetings");
    setFolderReturnTarget(undefined);
  }

  async function handleCreateFolder(name: string, description?: string) {
    try {
      const folder = await createFolder(name, description);
      dispatch({ type: "folderCreated", folder });
      return folder;
    } catch (err) {
      setError(messageFromError(err));
      return undefined;
    }
  }

  function handleFoldersImported(folders: FolderDto[]) {
    for (const folder of folders) {
      dispatch({ type: "folderCreated", folder });
    }
    toast.success(t("app.projects.added", { count: folders.length }));
  }

  async function handleRenameFolder(folderId: string, name: string, description?: string) {
    try {
      const folder = await renameFolder(folderId, name, description);
      dispatch({ type: "folderRenamed", folder });
    } catch (err) {
      setError(messageFromError(err));
    }
  }

  const { handleDeleteFolder, handleRemoveNoteFromFolder, handleSetNoteFolder } =
    createAppDomainActions({
      dispatch,
      noteSaveController,
      folderChangesRef,
      setError,
      state,
    });

  // Opening a note overlaps with the next open (fast clicks): only the newest
  // open's note is shown, and only its failure is reported.
  async function openLatestNote(noteId: string): Promise<NoteDto | undefined> {
    const request = ++latestNoteRequestRef.current;
    try {
      const note = await getNote(noteId);
      return request === latestNoteRequestRef.current ? note : undefined;
    } catch (err) {
      if (request === latestNoteRequestRef.current) throw err;
      return undefined;
    }
  }

  async function handleSelectNote(noteId: string) {
    try {
      const note = await openLatestNote(noteId);
      if (!note) return;
      dispatch({ type: "noteLoaded", note });
      setOriginFolderId(undefined);
      setOriginAllNotes(false);
      setFolderReturnTarget(undefined);
    } catch (err) {
      setError(messageFromError(err));
    }
  }

  // Drilling into a note from the All notes view. Mirrors the folder flow so
  // the note opens with a "Back to All notes" breadcrumb up top.
  async function handleSelectNoteFromAllNotes(noteId: string) {
    try {
      const note = await openLatestNote(noteId);
      if (!note) return;
      dispatch({ type: "noteLoaded", note });
      setOriginFolderId(undefined);
      setOriginAllNotes(true);
      setFolderReturnTarget(undefined);
      setActiveView("meetings");
    } catch (err) {
      setError(messageFromError(err));
    }
  }

  function handleEmptyNotesAfterDelete() {
    const currentView = activeViewRef.current;
    if (currentView === "meetings" || currentView === "notes" || currentView === "all-notes") {
      setActiveView("notes");
    }
    setOriginFolderId(undefined);
    setOriginAllNotes(false);
    setFolderReturnTarget(undefined);
  }

  const {
    handleDeleteNote,
    handleDeleteNotes,
    handleFlushNote,
    handleSaveNoteNow,
    handleSelectNoteFromFolder,
    handleUpdateNote,
  } = createNoteActions({
    dispatch,
    handleEmptyNotesAfterDelete,
    noteSaveController,
    pruneDeletedNoteTabs,
    setActiveView,
    setError,
    setFolderReturnTarget,
    setOriginAllNotes,
    setOriginFolderId,
    state,
  });

  const {
    handleStartRecording,
    meetingStartReadyRef,
    meetingStartListenerRegisteredRef,
    drainPendingMeetingStartRef,
    handleStartMeetingDetectedRecording,
    handleStartAgentRecording,
    handleOpenRecordingNote,
  } = useRecordingStartActions({
    activeViewRef,
    appBlocked,
    bootstrapped,
    calendarContextNotePartitionsRef,
    calendarContextNoteUpdatesRef,
    crossPartitionRecordingNoteIdRef,
    dispatch,
    handleEmptyNotesAfterDelete,
    pendingCalendarContextAdoptionsRef,
    recordingNoteIdRef,
    recordingStartInFlightRef,
    recordingStatusRef,
    selectedNoteId,
    selectedNoteIdRef,
    setActiveView,
    setCheckingSourceReadiness,
    setError,
    setFolderReturnTarget,
    setOriginAllNotes,
    setOriginFolderId,
    setRecordingNote,
    setSourceReadiness,
    sourceMode,
  });

  // Native retains the request and its terminal result until this webview
  // explicitly acknowledges it. Reloads can therefore replay the same token
  // without creating a second note or restarting an active capture.
  const meetingStartHandlerRef = useRef<(requestId: string, noteId: string) => Promise<boolean>>(
    async () => false,
  );
  meetingStartHandlerRef.current = handleStartMeetingDetectedRecording;

  useEffect(() => {
    if (appBlocked || !bootstrapped) return;
    drainPendingMeetingStartRef.current();
  }, [appBlocked, bootstrapped]);

  useRecordingEvents({
    drainPendingMeetingStartRef,
    meetingStartHandlerRef,
    meetingStartListenerRegisteredRef,
    meetingStartReadyRef,
    setError,
  });

  // The handler closes over frequently-changing state, but the Tauri listener
  // must register exactly once: re-subscribing tears the listener down and
  // events emitted in the gap are silently dropped (a dropped request costs
  // the agent a full proxy lease). The ref always holds the latest closure.
  const {
    applyNoteScopedProcessingFailure,
    handleFinishRecording,
    handlePauseRecording,
    handleResumeRecording,
  } = useRecordingControls({
    activeViewRef,
    appBlocked,
    bootstrapped,
    crossPartitionRecordingNoteIdRef,
    dispatch,
    finishingSessionsRef,
    handleStartAgentRecording,
    recordNoticesDemoRef,
    recordingNoteIdRef,
    recordingStatusRef,
    selectedNote,
    setActiveView,
    setError,
    setFolderReturnTarget,
    setOriginAllNotes,
    setOriginFolderId,
    setRecordingNote,
    setTabs,
    state,
    tabsRef,
  });

  meetingEndFinishHandlerRef.current = async (sessionId) => {
    if (!meetingEndReadyRef.current || !meetingEndListenerRegisteredRef.current) {
      return false;
    }
    const active = recordingStatusRef.current;
    if (!active || active.sessionId !== sessionId) {
      return true;
    }
    try {
      await handleFinishRecording(sessionId, { rethrow: true });
    } catch {
      // The normal finish handler has already surfaced the failure and the
      // saved-audio recovery path owns the artifact. A retained native request
      // is one-shot: retrying it could race recovery or another terminal path.
    }
    return true;
  };

  useMeetingEndEvents({
    drainPendingFinishRef: drainPendingMeetingEndFinishRef,
    finishHandlerRef: meetingEndFinishHandlerRef,
    listenerRegisteredRef: meetingEndListenerRegisteredRef,
    readyRef: meetingEndReadyRef,
    setStatus: setMeetingEndStatus,
  });

  useEffect(() => {
    if (!meetingEndReadyRef.current) return;
    drainPendingMeetingEndFinishRef.current();
  }, [appBlocked, bootstrapped]);

  useEffect(() => {
    if (meetingEndStatus?.phase !== "countdown") return;
    setMeetingEndNow(Date.now());
    const tick = window.setInterval(() => setMeetingEndNow(Date.now()), 1_000);
    return () => window.clearInterval(tick);
  }, [meetingEndStatus]);

  useEffect(() => {
    const evaluateLatestStatus = () => {
      const status = recordingTelemetryStore.getStatus();
      const now = Date.now();
      const meetingEndEligible = !!status && meetingEndStatus?.sessionId === status.sessionId;
      if (meetingEndEligible) {
        recordingInactivityTrackerRef.current = { sessionId: status.sessionId };
        if (recordingInactivityPrompt?.sessionId === status.sessionId) {
          setRecordingInactivityPrompt(null);
        }
        return;
      }
      const decision = nextRecordingInactivityDecision(
        recordingInactivityTrackerRef.current,
        status,
        now,
      );
      recordingInactivityTrackerRef.current = decision.tracker;

      if (
        recordingInactivityPrompt &&
        (!status ||
          status.sessionId !== recordingInactivityPrompt.sessionId ||
          status.state !== "recording" ||
          recordingHasActivity(status))
      ) {
        setRecordingInactivityPrompt(null);
        return;
      }

      if (
        !status ||
        !decision.shouldPrompt ||
        recordingInactivityPrompt?.sessionId === status.sessionId
      ) {
        return;
      }

      const prompt = {
        sessionId: status.sessionId,
        expiresAt: now + RECORDING_INACTIVITY_RESPONSE_MS,
      };
      setRecordingInactivityNow(now);
      setRecordingInactivityPrompt(prompt);
      void notifyRecordingStillMeetingPrompt(status.sessionId);
    };

    evaluateLatestStatus();
    return recordingTelemetryStore.subscribeStatus(evaluateLatestStatus);
  }, [
    recordingInactivityPrompt,
    recordingInactivityTrackerRef,
    recordingTelemetryStore,
    meetingEndStatus,
    setRecordingInactivityNow,
    setRecordingInactivityPrompt,
  ]);

  useEffect(() => {
    if (!recordingInactivityPrompt) return;

    setRecordingInactivityNow(Date.now());
    const tick = window.setInterval(() => {
      setRecordingInactivityNow(Date.now());
    }, 1000);
    const timeout = window.setTimeout(
      () => {
        const currentStatus = recordingStatusRef.current;
        const sessionId = recordingInactivityPrompt.sessionId;
        recordingInactivityTrackerRef.current = { sessionId };
        setRecordingInactivityPrompt(null);
        if (currentStatus?.sessionId !== sessionId || currentStatus.state !== "recording") {
          return;
        }
        void handlePauseRecording(sessionId).then((paused) => {
          if (paused) void notifyRecordingAutoPaused(sessionId);
        });
      },
      Math.max(0, recordingInactivityPrompt.expiresAt - Date.now()),
    );

    return () => {
      window.clearInterval(tick);
      window.clearTimeout(timeout);
    };
  }, [handlePauseRecording, recordingInactivityPrompt]);

  function handleKeepRecordingAfterInactivityPrompt() {
    const sessionId = recordingInactivityPrompt?.sessionId;
    if (sessionId) {
      recordingInactivityTrackerRef.current = {
        sessionId,
        snoozedUntil: Date.now() + RECORDING_INACTIVITY_SNOOZE_MS,
      };
    }
    setRecordingInactivityPrompt(null);
  }

  function handlePauseRecordingAfterInactivityPrompt() {
    const sessionId = recordingInactivityPrompt?.sessionId;
    if (!sessionId) return;
    recordingInactivityTrackerRef.current = { sessionId };
    setRecordingInactivityPrompt(null);
    void handlePauseRecording(sessionId);
  }

  const recordingInactivitySecondsRemaining = recordingInactivityPrompt
    ? Math.max(0, Math.ceil((recordingInactivityPrompt.expiresAt - recordingInactivityNow) / 1000))
    : 0;
  const meetingEndCountdown =
    meetingEndStatus?.phase === "countdown" &&
    meetingEndStatus.expiresAtMs !== undefined &&
    recordingStatusRef.current?.sessionId === meetingEndStatus.sessionId
      ? {
          sessionId: meetingEndStatus.sessionId,
          secondsRemaining: Math.max(
            0,
            Math.ceil((meetingEndStatus.expiresAtMs - meetingEndNow) / 1_000),
          ),
        }
      : null;

  function handleStopNowAfterMeetingEnd(sessionId: string) {
    void queueMeetingEndFinishRequest(sessionId).catch((err) => {
      setError(messageFromError(err));
    });
  }

  function handleKeepRecordingAfterMeetingEnd(sessionId: string) {
    void keepMeetingRecording(sessionId).catch((err) => {
      setError(messageFromError(err));
    });
  }

  const onboarding = renderAppOnboarding({ onboardingRequired, setOnboardingDone });
  if (onboarding) return onboarding;

  // The in-note RecorderBar covers the recording while you're looking at its
  // note. Elsewhere, the sidebar header carries a tiny recording presence; the
  // floating pill is only the collapsed-sidebar fallback.
  const viewingRecordingNote =
    activeView === "meetings" && selectedNoteId !== undefined && selectedNoteId === recordingNoteId;
  const recorderPresenceVisible = Boolean(state.recordingStatus) && !viewingRecordingNote;
  const recordingNoteTitle =
    (selectedNote?.id === recordingNoteId
      ? selectedNote?.title
      : state.notes.find((note) => note.id === recordingNoteId)?.title
    )?.trim() || t("app.newNote");
  // The dev console demo (window.__globalRecorderPill) force-shows the recorder
  // presence with synthetic status; otherwise it tracks the real recording.
  const recorderPresenceStatus =
    demoRecorderStatus ?? (recorderPresenceVisible ? state.recordingStatus : null);
  const sidebarRecorderStatus =
    recorderPresenceStatus && !sidebarCollapsed && activeView !== "settings"
      ? recorderPresenceStatus
      : null;
  const pillStatus =
    recorderPresenceStatus && !sidebarRecorderStatus ? recorderPresenceStatus : null;
  const pillIsDemo = demoRecorderStatus !== null;

  const workspaceContent = renderAppWorkspace({
    accessibilityStatus,
    checkingForUpdates,
    onCheckForUpdates: updatesEnabled ? () => runUpdateCheck("manual") : undefined,
    activeView,
    changeSettingsTab,
    checkingSourceReadiness,
    dispatch,
    folderReturnTarget,
    handleCreateFolder,
    handleCreateNote,
    handleDeleteFolder,
    handleDeleteNote,
    handleDeleteNotes,
    handleEnableAccessibility,
    handleEnableMicrophone,
    handleEnableSystemAudio,
    handleFinishRecording,
    handleKeepRecordingAfterMeetingEnd,
    handleFlushNote,
    handleFoldersImported,
    handlePauseRecording,
    handleRecovery,
    handleRemoveNoteFromFolder,
    handleRenameFolder,
    handleResumeRecording,
    handleSaveNoteNow,
    handleReturnToNote,
    handleSelectFolder,
    handleSelectNote,
    handleSelectNoteFromAllNotes,
    handleSelectNoteFromFolder,
    handleSetNoteFolder,
    handleSourceModeChange,
    handleStartRecording,
    handleStopNowAfterMeetingEnd,
    handleUpdateNote,
    memoryFolderFilter,
    meetingEndCountdown,
    microphoneBlocked,
    microphoneStatus,
    noteDetailScrollRef,
    noteToolbarActions,
    notesListRef,
    openMemorySettings,
    openTab,
    originAllNotes,
    originFolder,
    recordNoticesConsentPinned,
    recordingNoteId,
    selectedNote,
    selectedNoteId,
    selectedNoteLiveTranscript,
    selectedRecovery,
    setActiveView,
    setError,
    setFolderReturnTarget,
    setMoveDialogNoteIds,
    setOriginAllNotes,
    setOriginFolderId,
    setSettingsDetailPinned,
    setSettingsReturnView,
    setSettingsTab,
    settingsTab,
    sourceMode,
    sourceReadiness,
    state,
    takeNewTabIntent,
  });

  const appLayout = renderAppLayout({
    updateHub,
    accessibilityBannerDismissed,
    accessibilityBlocked,
    activateTab,
    activeTabId,
    activeView,
    changeSettingsTab,
    closeOtherTabs,
    closeTab,
    confirmDeleteNote,
    detailScrollerActive,
    dispatch,
    error,
    handleCreateFolder,
    handleDeleteNote,
    handleEnableAccessibility,
    handleKeepRecordingAfterInactivityPrompt,
    handleOpenRecordingNote,
    handlePauseRecordingAfterInactivityPrompt,
    handleRemoveNoteFromFolder,
    handleReorderTabs,
    handleSelectNote,
    handleSetNoteFolder,
    mainPanelBodyRef,
    moveDialogNoteIds,
    noteDetailScrollerActive,
    notesListRef,
    openNewTab,
    openSettings,
    openTab,
    pillIsDemo,
    pillStatus,
    recordingInactivityPrompt,
    recordingInactivitySecondsRemaining,
    recordingNoteTitle,
    selectedNote,
    setAccessibilityBannerDismissed,
    setActiveView,
    setConfirmDeleteNote,
    setError,
    setFolderReturnTarget,
    setMoveDialogNoteIds,
    setOriginAllNotes,
    setOriginFolderId,
    setSidebarCollapsed,
    setSidebarResizing,
    setSidebarTransition,
    setSidebarWidth,
    settingsDetailScrollerActive,
    settingsReturnView,
    settingsTab,
    sidebarCollapsed,
    sidebarRecorderStatus,
    sidebarResizing,
    sidebarTransition,
    sidebarWidth,
    state,
    tabItems,
    takeNewTabIntent,
    workspaceContent,
  });
  return (
    <RecordingTelemetryProvider store={recordingTelemetryStore}>
      {appLayout}
    </RecordingTelemetryProvider>
  );
}

// The collapsed transform is driven by `aria-pressed` on the parent button.
