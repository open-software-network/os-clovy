import { t } from "../i18n";
import { DictationHistoryView } from "../components/dictation/DictationHistoryView";
import { NotesList } from "../components/notes-list/NotesList";
import { BreadcrumbBar } from "../components/ui/BreadcrumbBar";
import { IconProjects } from "central-icons/IconProjects";
import { retryProcessing } from "../lib/tauri";
import { messageFromError } from "../lib/errors";
import type { RenderAppWorkspaceDependencies } from "./app-workspace-view-types";
import {
  ActivityTimelineViewRoute,
  AppSettingsRoute,
  FoldersWorkspaceRoute,
  NoteEditorRoute,
  RoutinesViewRoute,
} from "./workspace-lazy";

export function renderAppWorkspace(dependencies: RenderAppWorkspaceDependencies) {
  const {
    accessibilityStatus,
    checkingForUpdates,
    onCheckForUpdates,
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
  } = dependencies;

  return activeView === "settings" ? (
    <AppSettingsRoute
      onCheckForUpdates={onCheckForUpdates}
      checkingForUpdates={checkingForUpdates}
      sourceMode={sourceMode}
      sourceReadiness={sourceReadiness}
      checkingSourceReadiness={checkingSourceReadiness}
      microphonePermissionStatus={microphoneStatus}
      accessibilityPermissionStatus={accessibilityStatus}
      onSourceModeChange={handleSourceModeChange}
      onEnableMicrophone={handleEnableMicrophone}
      onEnableAccessibility={handleEnableAccessibility}
      onEnableSystemAudio={handleEnableSystemAudio}
      folders={state.folders}
      onFoldersImported={handleFoldersImported}
      memoryFolderFilter={memoryFolderFilter}
      onOpenProject={(folderId) => {
        handleSelectFolder(folderId);
        setActiveView("folders");
      }}
      activeTab={settingsTab}
      onTabChange={changeSettingsTab}
      onDetailPinnedChange={setSettingsDetailPinned}
    />
  ) : activeView === "dictation" ? (
    <DictationHistoryView
      onNavigateToSettings={(target) => {
        setSettingsReturnView(activeView);
        setActiveView("settings");
        setSettingsTab("dictation");
        const headingId = target === "style" ? "style-heading" : "dictionary-heading";
        window.setTimeout(() => {
          document.getElementById(headingId)?.scrollIntoView({
            behavior: "smooth",
            block: "start",
          });
        }, 80);
      }}
    />
  ) : activeView === "today" ? (
    <ActivityTimelineViewRoute
      onNavigateToSettings={(tab) => {
        setSettingsReturnView(activeView);
        setActiveView("settings");
        setSettingsTab(tab ?? "activity");
      }}
    />
  ) : activeView === "routines" ? (
    <RoutinesViewRoute />
  ) : activeView === "notes" || activeView === "all-notes" ? (
    <NotesList
      ref={notesListRef}
      notes={state.notes}
      activeRecordingNoteId={recordingNoteId}
      onSelectNote={(noteId) => {
        if (takeNewTabIntent()) {
          openTab({
            view: "meetings",
            noteId,
            originAllNotes: true,
          });
          return;
        }
        void handleSelectNoteFromAllNotes(noteId);
      }}
      onCreateNote={() => void handleCreateNote(null)}
      onOpenMoveDialog={(noteId) => setMoveDialogNoteIds([noteId])}
      onOpenMoveNotes={(noteIds) => setMoveDialogNoteIds(noteIds)}
      onDeleteNote={(noteId) => void handleDeleteNote(noteId)}
      onDeleteNotes={(noteIds) => void handleDeleteNotes(noteIds)}
    />
  ) : activeView === "folders" ? (
    <FoldersWorkspaceRoute
      folders={state.folders}
      notes={state.notes}
      selectedFolderId={state.selectedFolderId}
      folderBackTarget={
        folderReturnTarget
          ? {
              label: t("app.breadcrumb.backTo", { name: folderReturnTarget.label }),
              onBack: () => void handleReturnToNote(folderReturnTarget.noteId),
            }
          : undefined
      }
      onSelectFolder={(folderId) => handleSelectFolder(folderId)}
      onCreateFolder={(name, description) => handleCreateFolder(name, description)}
      onFoldersImported={handleFoldersImported}
      onRenameFolder={(folderId, name, description) =>
        handleRenameFolder(folderId, name, description)
      }
      onFolderUpdated={(folder) => dispatch({ type: "folderUpdated", folder })}
      onDeleteFolder={(folderId) => handleDeleteFolder(folderId)}
      onCreateNote={(folderId) => void handleCreateNote(folderId)}
      onSelectNote={(noteId) => {
        const folderId = state.selectedFolderId;
        if (takeNewTabIntent()) {
          openTab({
            view: "meetings",
            noteId,
            originFolderId: folderId,
          });
          return;
        }
        if (folderId) {
          void handleSelectNoteFromFolder(noteId, folderId);
        } else {
          void handleSelectNote(noteId).then(() => setActiveView("meetings"));
        }
      }}
      onAssignNoteToFolder={(noteId, folderId) =>
        handleSetNoteFolder(noteId, folderId, { rethrow: true })
      }
      onRemoveNoteFromFolder={(noteId, folderId) =>
        void handleRemoveNoteFromFolder(noteId, folderId)
      }
      onOpenMoveDialog={(noteId) => setMoveDialogNoteIds([noteId])}
      onDeleteNote={(noteId) => void handleDeleteNote(noteId)}
      onManageProjectMemory={(folderId) => openMemorySettings(folderId)}
    />
  ) : selectedNote ? (
    <div className="note-shell">
      {/* Every note gets the toolbar so its content starts at a
                        consistent height and
                        the note actions live in one predictable spot. The left
                        shows breadcrumb nav when there's a parent, else a quiet
                        "Notes" root. */}
      {originFolder ? (
        <BreadcrumbBar
          backLabel={t("app.breadcrumb.backTo", { name: originFolder.name })}
          onBack={() => {
            setActiveView("folders");
            dispatch({
              type: "folderSelected",
              folderId: originFolder.id,
            });
            setOriginFolderId(undefined);
          }}
          items={[
            {
              label: originFolder.name,
              icon: <IconProjects size={13} />,
              onClick: () => {
                setActiveView("folders");
                dispatch({
                  type: "folderSelected",
                  folderId: originFolder.id,
                });
                setOriginFolderId(undefined);
              },
            },
            {
              label: selectedNote.title.trim() || t("app.newNote"),
            },
          ]}
          actions={noteToolbarActions}
        />
      ) : originAllNotes ? (
        <BreadcrumbBar
          backLabel={t("app.breadcrumb.backToMeetingNotes")}
          onBack={() => {
            setActiveView("all-notes");
            setOriginAllNotes(false);
          }}
          items={[
            {
              label: t("app.nav.meetingNotes"),
              onClick: () => {
                setActiveView("all-notes");
                setOriginAllNotes(false);
              },
            },
            {
              label: selectedNote.title.trim() || t("app.newNote"),
            },
          ]}
          actions={noteToolbarActions}
        />
      ) : (
        <BreadcrumbBar
          items={[
            { label: t("app.nav.notes"), onClick: () => setActiveView("all-notes") },
            {
              label: selectedNote.title.trim() || t("app.newNote"),
            },
          ]}
          actions={noteToolbarActions}
        />
      )}
      <div ref={noteDetailScrollRef} className="note-detail-scroll" data-has-detail-bar="true">
        <NoteEditorRoute
          note={selectedNote}
          transcriptScrollRef={noteDetailScrollRef}
          folders={state.folders}
          recordingStatus={selectedNoteId === recordingNoteId ? state.recordingStatus : undefined}
          meetingEndCountdown={selectedNoteId === recordingNoteId ? meetingEndCountdown : null}
          recordingDisabled={Boolean(state.recordingStatus && selectedNoteId !== recordingNoteId)}
          liveTranscript={selectedNoteLiveTranscript}
          sourceMode={sourceMode}
          sourceReadiness={sourceReadiness}
          recovery={selectedRecovery}
          onRecoverRecording={(sessionId) => handleRecovery(sessionId, "validate")}
          onDiscardRecording={(sessionId) => handleRecovery(sessionId, "discard")}
          onTitleChange={(title) => handleUpdateNote(selectedNote.id, { title })}
          onContentChange={(sourceNoteId, editedContent) => {
            handleUpdateNote(sourceNoteId, { editedContent });
          }}
          onFlushNote={(noteId) => void handleFlushNote(noteId)}
          onSourceModeChange={handleSourceModeChange}
          onEnableSystemAudio={handleEnableSystemAudio}
          onEnableMicrophone={handleEnableMicrophone}
          microphoneBlocked={microphoneBlocked}
          consentReminderPinned={
            import.meta.env.DEV && recordNoticesConsentPinned && selectedNoteId === recordingNoteId
          }
          onTabChange={(activeTab) =>
            void handleSaveNoteNow(selectedNote.id, {
              activeTab,
            })
          }
          onStartRecording={(video) => void handleStartRecording(video)}
          onPauseRecording={(sessionId) => void handlePauseRecording(sessionId)}
          onResumeRecording={(sessionId) => void handleResumeRecording(sessionId)}
          onFinishRecording={(sessionId) => void handleFinishRecording(sessionId)}
          onKeepRecordingAfterMeetingEnd={handleKeepRecordingAfterMeetingEnd}
          onStopNowAfterMeetingEnd={handleStopNowAfterMeetingEnd}
          onRetry={async () => {
            if (!selectedNote) return;
            try {
              const note = await retryProcessing(
                selectedNote.id,
                selectedNote.retryRecordingSessionId,
              );
              dispatch({ type: "noteProcessingUpdated", note });
            } catch (err) {
              const message = messageFromError(err);
              dispatch({
                type: "noteProcessingUpdated",
                note: {
                  ...selectedNote,
                  processingStatus: "failed",
                  lastError: message,
                },
              });
              setError(null);
              throw err;
            }
          }}
          onAssignFolder={(folderId) => void handleSetNoteFolder(selectedNote.id, folderId)}
          onRemoveFolder={(folderId) => void handleRemoveNoteFromFolder(selectedNote.id, folderId)}
          onNavigateToFolder={(folderId) => {
            setActiveView("folders");
            dispatch({ type: "folderSelected", folderId });
            setFolderReturnTarget({
              noteId: selectedNote.id,
              label: selectedNote.title.trim() || t("app.newNote"),
            });
            setOriginFolderId(undefined);
          }}
          onCreateAndAssignFolder={(name) => {
            void (async () => {
              const folder = await handleCreateFolder(name);
              if (folder) {
                await handleSetNoteFolder(selectedNote.id, folder.id);
              }
            })();
          }}
        />
      </div>
    </div>
  ) : (
    <section className="editor-empty" aria-label={t("app.workspace.openingNote")} />
  );
}
