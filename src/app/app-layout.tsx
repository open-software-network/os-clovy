import { AnimatePresence, motion } from "framer-motion";
import type { CSSProperties } from "react";
import { t } from "../i18n";
import { MoveNoteToFolderDialog } from "../components/folders/MoveNoteToFolderDialog";
import { GlobalRecorderPill } from "../components/recorder/GlobalRecorderPill";
import { PermissionBanner } from "../components/permissions/PermissionBanner";
import { McpApprovalIndicator } from "../components/settings/McpApprovalIndicator";
import { Sidebar } from "../components/sidebar/Sidebar";
import { TabBar } from "../components/tabs/TabBar";
import { ConnectorApprovalsTray } from "../components/connectors/ConnectorApprovalsTray";
import { ComputerUseApprovalsTray } from "../components/agent/ComputerUseApprovalsTray";
import { BrowserSessionsTray } from "../components/agent/BrowserSessionsTray";
import { Dialog } from "../components/ui/Dialog";
import { isWindowsPlatform } from "../lib/platform";
import type { NoteListItemDto } from "../lib/tauri";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { handleSidebarResizeStart } from "./sidebar-resize";
import { SidebarToggleGlyph, handleTitlebarPointerDown } from "./app-helpers";
import {
  SIDEBAR_COLLAPSE_WIDTH,
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MIN_WIDTH,
  sidebarMaxWidth,
} from "./app-shell";
import { UpdateHub } from "./app-effects/update-ui";
import type { RenderAppLayoutDependencies } from "./app-layout-types";

export function renderAppLayout(dependencies: RenderAppLayoutDependencies) {
  const {
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
  } = dependencies;
  return (
    <main
      className={["app-shell", activeView !== "settings" ? "app-shell-sidebar-default" : ""]
        .filter(Boolean)
        .join(" ")}
      data-platform={isWindowsPlatform() ? "windows" : undefined}
      data-sidebar={sidebarCollapsed ? "collapsed" : "expanded"}
      data-sidebar-resizing={sidebarResizing ? "true" : "false"}
      data-sidebar-transition={sidebarTransition}
      style={
        {
          // The grid columns read this directly, so collapsed must pin it to 0
          // (the stored width is preserved for the next expand). During a drag
          // the resize logic overrides it imperatively.
          "--sidebar-w-current": `${sidebarCollapsed ? 0 : sidebarWidth}px`,
        } as CSSProperties
      }
    >
      <div
        className="titlebar-drag"
        aria-hidden
        data-tauri-drag-region
        onPointerDown={handleTitlebarPointerDown}
      />
      <button
        type="button"
        className="chrome-sidebar-toggle"
        aria-label={sidebarCollapsed ? t("app.sidebar.show") : t("app.sidebar.hide")}
        aria-pressed={sidebarCollapsed}
        onClick={() => {
          setSidebarTransition("none");
          if (sidebarCollapsed) {
            setSidebarWidth((width) => Math.max(width, SIDEBAR_DEFAULT_WIDTH));
            setSidebarCollapsed(false);
            return;
          }
          setSidebarCollapsed(true);
        }}
      >
        <SidebarToggleGlyph />
      </button>
      <Sidebar
        notes={state.notes}
        activeView={activeView}
        settingsTab={settingsTab}
        onSettingsTabChange={changeSettingsTab}
        onChangeView={(view) => {
          if (takeNewTabIntent()) {
            openTab({ view });
            return;
          }
          if (view === "settings") openSettings();
          else setActiveView(view);
          if (view === "folders") {
            setFolderReturnTarget(undefined);
            dispatch({ type: "folderSelected", folderId: undefined });
          }
          if (view !== "meetings" && view !== "notes") {
            setOriginFolderId(undefined);
            setOriginAllNotes(false);
            setFolderReturnTarget(undefined);
          }
        }}
        onExitSettings={() => setActiveView(settingsReturnView)}
        onSelectNote={(noteId) => {
          if (takeNewTabIntent()) {
            openTab({ view: "meetings", noteId });
            return;
          }
          void handleSelectNote(noteId);
        }}
        recordingStatus={sidebarRecorderStatus}
        recordingTitle={recordingNoteTitle}
        onOpenRecording={() => (pillIsDemo ? undefined : void handleOpenRecordingNote())}
        collapsed={sidebarCollapsed}
        footerAccessory={
          <>
            {!sidebarCollapsed ? <McpApprovalIndicator onOpen={() => openSettings("mcp")} /> : null}
            <UpdateHub {...updateHub} />
          </>
        }
      />
      <div
        className="sidebar-resize-handle"
        role="separator"
        aria-orientation="vertical"
        aria-label={t("app.sidebar.resize")}
        onPointerDown={(event) =>
          handleSidebarResizeStart(event, sidebarWidth, {
            collapseWidth: SIDEBAR_COLLAPSE_WIDTH,
            minWidth: SIDEBAR_MIN_WIDTH,
            maxWidth: sidebarMaxWidth,
            onStart: () => {
              setSidebarResizing(true);
              setSidebarTransition("none");
            },
            onEnd: (finalWidth) => {
              if (finalWidth <= SIDEBAR_COLLAPSE_WIDTH) {
                setSidebarResizing(false);
                setSidebarTransition("smooth");
                setSidebarWidth(Math.max(0, finalWidth));
                setSidebarCollapsed(true);
                return;
              }
              const nextWidth = Math.min(
                sidebarMaxWidth(),
                Math.max(SIDEBAR_MIN_WIDTH, finalWidth),
              );
              setSidebarResizing(false);
              setSidebarCollapsed(false);
              setSidebarWidth(nextWidth);
            },
          })
        }
      />
      <div className="main-column">
        <TabBar
          tabs={tabItems}
          activeTabId={activeTabId}
          onActivate={activateTab}
          onClose={closeTab}
          onCloseOthers={closeOtherTabs}
          onNew={openNewTab}
          onReorder={handleReorderTabs}
          layoutFrozen={sidebarResizing}
          onDragRegionPointerDown={handleTitlebarPointerDown}
        />
        <section className="main-panel">
          {accessibilityBlocked && !accessibilityBannerDismissed ? (
            <PermissionBanner
              onDismiss={() => setAccessibilityBannerDismissed(true)}
              onEnableAccessibility={handleEnableAccessibility}
            />
          ) : null}
          <div
            ref={mainPanelBodyRef}
            className="main-panel-body"
            data-active-view={activeView}
            data-detail-scroller={detailScrollerActive ? "true" : undefined}
            data-note-detail-scroller={
              noteDetailScrollerActive || settingsDetailScrollerActive ? "true" : undefined
            }
          >
            {error ? <p className="error-banner">{error}</p> : null}
            <div className="workspace">{workspaceContent}</div>
          </div>
          <AnimatePresence>
            {pillStatus ? (
              <motion.div
                key="global-recorder"
                className="global-recorder-dock"
                initial={{ opacity: 0, y: -8 }}
                animate={{
                  opacity: 1,
                  y: 0,
                  transition: { duration: 0.22, ease: [0.22, 1, 0.36, 1] },
                }}
                exit={{
                  opacity: 0,
                  y: -8,
                  transition: { duration: 0.14, ease: [0.22, 1, 0.36, 1] },
                }}
              >
                <GlobalRecorderPill
                  status={pillStatus}
                  title={recordingNoteTitle}
                  onOpen={() => (pillIsDemo ? undefined : void handleOpenRecordingNote())}
                />
              </motion.div>
            ) : null}
          </AnimatePresence>
        </section>
        <ConfirmDialog
          open={confirmDeleteNote && !!selectedNote}
          onClose={() => setConfirmDeleteNote(false)}
          onConfirm={async () => {
            setConfirmDeleteNote(false);
            if (selectedNote) await handleDeleteNote(selectedNote.id);
          }}
          title={t("app.deleteNote.title")}
          description={t("app.deleteNote.description")}
          confirmLabel={t("app.deleteNote.confirm")}
          destructive
        />
      </div>
      <Dialog
        open={recordingInactivityPrompt !== null}
        onClose={handleKeepRecordingAfterInactivityPrompt}
        title={t("app.inactivity.title")}
        description={t("app.inactivity.description")}
        width={420}
        footer={
          <>
            <button
              type="button"
              className="primary-action"
              onClick={handlePauseRecordingAfterInactivityPrompt}
            >
              {t("app.inactivity.pause")}
            </button>
            <button
              type="button"
              className="primary-action primary-solid"
              onClick={handleKeepRecordingAfterInactivityPrompt}
            >
              {t("app.inactivity.keep")}
            </button>
          </>
        }
      >
        <div className="dialog-body">
          <p className="recording-inactivity-copy">
            {t("app.inactivity.countdown", { count: recordingInactivitySecondsRemaining })}
          </p>
        </div>
      </Dialog>
      <MoveNoteToFolderDialog
        open={moveDialogNoteIds !== null}
        onClose={() => setMoveDialogNoteIds(null)}
        notes={
          moveDialogNoteIds
            ? moveDialogNoteIds
                .map((id) => state.notes.find((n) => n.id === id))
                .filter((note): note is NoteListItemDto => note !== undefined)
            : []
        }
        folders={state.folders}
        onSetFolder={(noteId, folderId) => handleSetNoteFolder(noteId, folderId, { rethrow: true })}
        onCreateFolder={(name) => handleCreateFolder(name)}
        onRemoveFolder={(noteId, folderId) =>
          handleRemoveNoteFromFolder(noteId, folderId, { rethrow: true })
        }
        onMoved={() => notesListRef.current?.resetSelection()}
      />
      <div className="shell-approvals-stack">
        {sidebarCollapsed ? (
          <McpApprovalIndicator floating onOpen={() => openSettings("mcp")} />
        ) : null}
        <BrowserSessionsTray />
        <ComputerUseApprovalsTray />
        <ConnectorApprovalsTray />
      </div>
    </main>
  );
}
