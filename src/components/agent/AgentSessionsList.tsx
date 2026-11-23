import { IconCheckmark2Medium } from "central-icons-filled/IconCheckmark2Medium";
import { IconArchive } from "central-icons/IconArchive";
import { IconArrowUndoUp } from "central-icons/IconArrowUndoUp";
import { IconBubble3 } from "central-icons/IconBubble3";
import { IconCrossMedium } from "central-icons/IconCrossMedium";
import { IconDotGrid1x3Horizontal } from "central-icons/IconDotGrid1x3Horizontal";
import { IconFolderAddRight } from "central-icons/IconFolderAddRight";
import { IconFolderDelete } from "central-icons/IconFolderDelete";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { IconMoveFolder } from "central-icons/IconMoveFolder";
import { IconPencil } from "central-icons/IconPencil";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { IconTrashCan } from "central-icons/IconTrashCan";
import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";
import { type TFunction, useT } from "../../i18n";
import { formatListTimestamp } from "../../lib/date-format";
import { AGENT_DELETE_SESSION_EVENT } from "../../lib/agent-events";
import { messageFromError } from "../../lib/errors";
import { useForcedEmptyStates } from "../../lib/empty-states-demo";
import { primaryShortcutLabel } from "../../lib/platform";
import { useClampedMenuTop } from "../../lib/use-clamped-menu-top";
import { deleteAgentSession, type FolderDto } from "../../lib/tauri";
import type { AgentSessionDto } from "../../lib/agent-runtime-contract";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { EmptyState } from "../ui/EmptyState";
import { Select } from "../ui/Select";
import { RenameSessionDialog } from "./RenameSessionDialog";

type AgentSessionsListProps = {
  sessions: AgentSessionDto[];
  folders: FolderDto[];
  /** sessionId -> project (folder) ids the session is filed under. */
  sessionFolderIds: Record<string, string[]>;
  workingSessionIds?: ReadonlySet<string>;
  waitingSessionIds?: ReadonlySet<string>;
  /** stored session id -> completed_at ISO (JUN-203). "Archived" is the
   * user-facing term; these sessions leave the default Active lens and live
   * under the Archived status filter. */
  completedSessionIds?: Record<string, string>;
  onToggleCompleted?: (sessionId: string, completed: boolean) => void;
  onSelectSession: (session: AgentSessionDto) => void;
  onNewSession: () => void;
  /** stored session id (not the runtime session id). */
  onRenameSession: (sessionId: string, title: string) => void;
  onOpenMoveDialog: (sessionId: string) => void;
  onOpenMoveSessions: (sessionIds: string[]) => void;
  onRemoveFromProject: (sessionId: string, folderId: string) => void;
};

export type AgentSessionsListHandle = {
  resetSelection: () => void;
};

/** The status lens on the one sessions list: active by default, the archive
 * one flip away. Archived is the user-facing term for the persisted
 * completed_at mark (JUN-203). */
type SessionStatusFilter = "active" | "archived" | "all";

function statusFilterOptions(t: TFunction) {
  return [
    { value: "active", label: t("agent.sessions.filter.active") },
    { value: "archived", label: t("agent.sessions.filter.archived") },
    { value: "all", label: t("agent.sessions.filter.all") },
  ];
}

const EMPTY_SESSION_IDS: ReadonlySet<string> = new Set();
const NO_SESSIONS: AgentSessionDto[] = [];
const EMPTY_COMPLETED: Record<string, string> = {};

export const AgentSessionsList = forwardRef<AgentSessionsListHandle, AgentSessionsListProps>(
  function AgentSessionsList(
    {
      sessions: allSessions,
      folders,
      sessionFolderIds,
      workingSessionIds = EMPTY_SESSION_IDS,
      waitingSessionIds = EMPTY_SESSION_IDS,
      completedSessionIds = EMPTY_COMPLETED,
      onToggleCompleted,
      onSelectSession,
      onNewSession,
      onRenameSession,
      onOpenMoveDialog,
      onOpenMoveSessions,
      onRemoveFromProject,
    },
    ref,
  ) {
    const t = useT();
    // __emptyStates() preview (dev console): render the page as a fresh
    // install would see it, real data untouched underneath.
    const sessions = useForcedEmptyStates() ? NO_SESSIONS : allSessions;
    const [query, setQuery] = useState("");
    const newSessionShortcut = primaryShortcutLabel("N");
    const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
    const [confirmBulkDelete, setConfirmBulkDelete] = useState(false);
    const [bulkDeleteError, setBulkDeleteError] = useState<string | null>(null);
    const [statusFilter, setStatusFilter] = useState<SessionStatusFilter>("active");
    const [exit, setExit] = useState<null | "slide" | "fade">(null);
    const exitCauseRef = useRef<"slide" | "fade">("fade");

    const sortedSessions = useMemo(
      () =>
        [...sessions].sort((a, b) => {
          const statusDelta =
            sessionStatusPriority(b.id, workingSessionIds, waitingSessionIds) -
            sessionStatusPriority(a.id, workingSessionIds, waitingSessionIds);
          if (statusDelta !== 0) return statusDelta;
          return b.updatedAt.localeCompare(a.updatedAt);
        }),
      [sessions, waitingSessionIds, workingSessionIds],
    );
    const filteredSessions = useMemo(() => {
      const normalized = query.trim().toLowerCase();
      if (!normalized) return sortedSessions;
      return sortedSessions.filter((session) =>
        `${session.title} ${sessionStatusLabel(
          sessionStatus(session.id, workingSessionIds, waitingSessionIds),
          t,
        )}`
          .toLowerCase()
          .includes(normalized),
      );
    }, [sortedSessions, query, waitingSessionIds, workingSessionIds, t]);
    const activeSessions = useMemo(
      () => filteredSessions.filter((session) => !completedSessionIds[session.id]),
      [filteredSessions, completedSessionIds],
    );
    const archivedSessions = useMemo(
      () =>
        filteredSessions
          .filter((session) => Boolean(completedSessionIds[session.id]))
          .sort((a, b) =>
            (completedSessionIds[b.id] ?? "").localeCompare(completedSessionIds[a.id] ?? ""),
          ),
      [filteredSessions, completedSessionIds],
    );
    // The rows the current lens shows: active only (default), the archive
    // (most recently archived first), or everything in one list with the
    // archived rows visually receded.
    const listedSessions =
      statusFilter === "active"
        ? activeSessions
        : statusFilter === "archived"
          ? archivedSessions
          : filteredSessions;
    // Bulk selection acts on the UNFILTERED set of non-completed sessions: a
    // completed session is never counted/moved/deleted (JUN-203 review), but a
    // selection must survive a search that hides it — like NotesList, the search
    // query only affects what's visible, not what's selected. Completed sessions
    // are excluded here (not just from the visible list) so a session marked
    // complete while selected leaves the selection.
    const selectedSessionIds = useMemo(
      () =>
        sortedSessions
          .filter((session) => selectedIds.has(session.id) && !completedSessionIds[session.id])
          .map((session) => session.id),
      [sortedSessions, selectedIds, completedSessionIds],
    );
    const visibleSelectedCount = activeSessions.filter((session) =>
      selectedIds.has(session.id),
    ).length;
    const selectedCount = selectedSessionIds.length;
    const hasUnselectedVisibleSessions =
      activeSessions.length > 0 && visibleSelectedCount < activeSessions.length;

    const lastCountRef = useRef(selectedCount);
    if (selectedCount > 0) lastCountRef.current = selectedCount;
    const displayCount = selectedCount > 0 ? selectedCount : lastCountRef.current;

    const previousCountRef = useRef(0);
    useEffect(() => {
      const previousCount = previousCountRef.current;
      previousCountRef.current = selectedCount;
      if (selectedCount > 0) {
        setExit(null);
        return;
      }
      if (previousCount === 0) return;
      const cause = exitCauseRef.current;
      exitCauseRef.current = "fade";
      setExit((current) => current ?? cause);
    }, [selectedCount]);

    const barMounted = selectedCount > 0 || exit !== null;
    const isExiting = selectedCount === 0 && exit !== null;

    // Prune the selection against the UNFILTERED session set: drop ids that were
    // deleted or marked complete (completing a selected session must not leave it
    // in the bulk selection), but never drop a session merely because the current
    // search hides it — the selection persists across searches.
    useEffect(() => {
      const keepIds = new Set(
        sessions.filter((session) => !completedSessionIds[session.id]).map((session) => session.id),
      );
      setSelectedIds((previous) => {
        const next = new Set([...previous].filter((id) => keepIds.has(id)));
        return next.size === previous.size ? previous : next;
      });
      if (sessions.length === 0) {
        setConfirmBulkDelete(false);
        setBulkDeleteError(null);
      }
    }, [sessions, completedSessionIds]);

    function toggleSelected(sessionId: string) {
      setSelectedIds((previous) => {
        const next = new Set(previous);
        if (next.has(sessionId)) next.delete(sessionId);
        else next.add(sessionId);
        return next;
      });
    }

    const resetSelection = useCallback(() => {
      exitCauseRef.current = "slide";
      setSelectedIds(new Set());
      setConfirmBulkDelete(false);
      setBulkDeleteError(null);
    }, []);

    useImperativeHandle(ref, () => ({ resetSelection }), [resetSelection]);

    function selectAllVisibleSessions() {
      setSelectedIds((previous) => {
        const next = new Set(previous);
        for (const session of activeSessions) {
          next.add(session.id);
        }
        return next;
      });
    }

    function deselectAllVisibleSessions() {
      setSelectedIds((previous) => {
        const next = new Set(previous);
        for (const session of activeSessions) {
          next.delete(session.id);
        }
        return next;
      });
    }

    useEffect(() => {
      if (selectedCount === 0 || confirmBulkDelete) return;
      function onKey(event: KeyboardEvent) {
        const target = event.target instanceof Element ? event.target : null;
        if (event.key === "Escape" && !target?.closest('[role="dialog"]')) resetSelection();
      }
      window.addEventListener("keydown", onKey);
      return () => window.removeEventListener("keydown", onKey);
    }, [selectedCount, confirmBulkDelete, resetSelection]);

    async function handleBulkDelete() {
      try {
        for (const sessionId of selectedSessionIds) {
          await deleteAgentSession(sessionId);
          window.setTimeout(() => {
            window.dispatchEvent(
              new CustomEvent(AGENT_DELETE_SESSION_EVENT, {
                detail: { sessionId },
              }),
            );
          }, 0);
        }
        resetSelection();
      } catch (err) {
        setBulkDeleteError(messageFromError(err));
        throw err;
      }
    }

    return (
      <section
        className="all-notes-workspace agent-sessions-workspace"
        aria-label={t("agent.sessions.title")}
      >
        <header className="folders-header">
          <div className="folders-heading">
            <h1>
              {t("agent.sessions.title")}
              {sessions.length > 0 ? (
                <span className="folders-count">{sessions.length}</span>
              ) : null}
            </h1>
          </div>
          <button
            type="button"
            className="primary-action primary-solid folders-create"
            onClick={onNewSession}
          >
            <IconPlusMedium size={13} />
            {t("agent.sessions.new")}
            <kbd className="primary-action-kbd" aria-hidden>
              {newSessionShortcut}
            </kbd>
          </button>
        </header>

        {sessions.length > 0 ? (
          <div className="folders-controls">
            <label className="folders-search">
              <IconMagnifyingGlass size={14} />
              <input
                type="search"
                placeholder={t("common.search")}
                value={query}
                onChange={(event) => setQuery(event.currentTarget.value)}
              />
            </label>
            <Select
              className="agent-sessions-filter"
              value={statusFilter}
              options={statusFilterOptions(t)}
              placeholder={t("agent.sessions.filter.active")}
              ariaLabel={t("agent.sessions.filterLabel")}
              popoverWidth={140}
              onChange={(value) => {
                const next = value as SessionStatusFilter;
                if (next !== statusFilter) resetSelection();
                setStatusFilter(next);
              }}
            />
          </div>
        ) : null}

        {sessions.length === 0 ? (
          <EmptyState
            label={t("agent.sessions.empty.label")}
            icon={<IconBubble3 size={28} />}
            title={t("agent.sessions.empty.title")}
            description={t("agent.sessions.empty.description")}
            action={
              <button type="button" className="primary-action primary-solid" onClick={onNewSession}>
                <IconPlusMedium size={13} />
                {t("agent.sessions.empty.label")}
              </button>
            }
          />
        ) : listedSessions.length === 0 ? (
          <div className="folders-empty">
            <p>
              {query.trim()
                ? t("agent.sessions.noMatch", { query: query.trim() })
                : statusFilter === "archived"
                  ? t("agent.sessions.noArchived")
                  : statusFilter === "all"
                    ? t("agent.sessions.noSessions")
                    : t("agent.sessions.noActive")}
            </p>
          </div>
        ) : (
          <ul
            className="folder-notes all-notes-list"
            role="list"
            data-selecting={selectedCount > 0}
          >
            {listedSessions.map((session) => {
              const archived = Boolean(completedSessionIds[session.id]);
              return (
                <AgentSessionListRow
                  key={session.id}
                  session={session}
                  projectName={projectNameFor(session.id, sessionFolderIds, folders)}
                  currentFolderId={sessionFolderIds[session.id]?.[0]}
                  status={sessionStatus(session.id, workingSessionIds, waitingSessionIds)}
                  checked={selectedIds.has(session.id)}
                  archived={archived}
                  archivedLabel={archived && statusFilter === "all"}
                  onToggleArchived={
                    onToggleCompleted ? () => onToggleCompleted(session.id, !archived) : undefined
                  }
                  onToggleSelected={() => toggleSelected(session.id)}
                  onSelect={() => onSelectSession(session)}
                  onRename={(title) => onRenameSession(session.id, title)}
                  onOpenMove={() => onOpenMoveDialog(session.id)}
                  onRemoveFromProject={(folderId) => onRemoveFromProject(session.id, folderId)}
                />
              );
            })}
          </ul>
        )}

        {barMounted ? (
          <div
            className="meetings-bulk-bar"
            role="toolbar"
            aria-label={t("agent.sessions.selection")}
            data-exit={isExiting ? exit : undefined}
            onAnimationEnd={(event) => {
              if (!isExiting || event.target !== event.currentTarget) return;
              if (event.animationName && !event.animationName.startsWith("meetings-bulk-bar-out")) {
                return;
              }
              setExit(null);
            }}
          >
            <span className="meetings-bulk-count">
              {t("agent.sessions.selectedCount", { count: displayCount })}
            </span>
            {hasUnselectedVisibleSessions ? (
              <button
                type="button"
                className="meetings-bulk-action"
                onClick={selectAllVisibleSessions}
                disabled={isExiting}
              >
                {t("agent.sessions.selectAll")}
              </button>
            ) : null}
            <button
              type="button"
              className="meetings-bulk-action"
              onClick={deselectAllVisibleSessions}
              disabled={isExiting}
            >
              {t("agent.sessions.deselectAll")}
            </button>
            <button
              type="button"
              className="meetings-bulk-action"
              onClick={() => onOpenMoveSessions(selectedSessionIds)}
              disabled={isExiting}
            >
              {t("agent.sessions.move")}
            </button>
            <button
              type="button"
              className="meetings-bulk-action"
              onClick={() => setConfirmBulkDelete(true)}
              disabled={isExiting}
            >
              {t("common.delete")}
            </button>
            <button
              type="button"
              className="meetings-bulk-dismiss"
              aria-label={t("agent.sessions.clearSelection")}
              onClick={resetSelection}
              disabled={isExiting}
            >
              <IconCrossMedium size={14} />
            </button>
          </div>
        ) : null}

        <ConfirmDialog
          open={confirmBulkDelete}
          onClose={() => {
            setConfirmBulkDelete(false);
            setBulkDeleteError(null);
          }}
          onConfirm={() => handleBulkDelete()}
          title={t("agent.sessions.bulkDeleteTitle", { count: selectedCount })}
          description={bulkDeleteError || t("agent.sessions.bulkDeleteDescription")}
          confirmLabel={t("agent.sessions.deleteSessions", { count: selectedCount })}
          destructive
        />
      </section>
    );
  },
);

function projectNameFor(
  sessionId: string,
  sessionFolderIds: Record<string, string[]>,
  folders: FolderDto[],
) {
  const folderId = sessionFolderIds[sessionId]?.[0];
  if (!folderId) return undefined;
  return folders.find((folder) => folder.id === folderId)?.name;
}

type AgentSessionListStatus = "running" | "waitingForUser" | undefined;

function sessionStatus(
  sessionId: string,
  workingSessionIds: ReadonlySet<string>,
  waitingSessionIds: ReadonlySet<string>,
): AgentSessionListStatus {
  if (waitingSessionIds.has(sessionId)) return "waitingForUser";
  if (workingSessionIds.has(sessionId)) return "running";
  return undefined;
}

function sessionStatusPriority(
  sessionId: string,
  workingSessionIds: ReadonlySet<string>,
  waitingSessionIds: ReadonlySet<string>,
) {
  if (waitingSessionIds.has(sessionId)) return 2;
  if (workingSessionIds.has(sessionId)) return 1;
  return 0;
}

function sessionStatusLabel(status: AgentSessionListStatus, t: TFunction) {
  if (status === "waitingForUser") return t("agent.sessions.status.needsYou");
  if (status === "running") return t("agent.sessions.status.working");
  return "";
}

function AgentSessionListRow({
  session,
  projectName,
  currentFolderId,
  status,
  checked,
  archived,
  archivedLabel,
  onToggleArchived,
  onToggleSelected,
  onSelect,
  onRename,
  onOpenMove,
  onRemoveFromProject,
}: {
  session: AgentSessionDto;
  projectName?: string;
  currentFolderId?: string;
  status?: AgentSessionListStatus;
  checked: boolean;
  archived?: boolean;
  /** Say "Archived" in the row's meta line — only where the lens mixes
   * statuses (the All filter); in the Archived lens the filter is the label,
   * matching the archive-view consensus (ChatGPT, Telegram, Slack Later). */
  archivedLabel?: boolean;
  onToggleArchived?: () => void;
  onToggleSelected: () => void;
  onSelect: () => void;
  onRename: (title: string) => void;
  onOpenMove: () => void;
  onRemoveFromProject: (folderId: string) => void;
}) {
  const t = useT();
  const title = session.title.trim() || t("agent.sessions.untitled");
  const preview =
    session.source === "legacy_routine"
      ? t("agent.sessions.preview.routine")
      : t("agent.sessions.preview.conversation");
  const subtitle = [archivedLabel ? t("agent.sessions.archivedMeta") : null, projectName, preview]
    .filter(Boolean)
    .join(" · ");
  const statusLabel = sessionStatusLabel(status, t);
  const [menu, setMenu] = useState<{ right: number; top: number } | null>(null);
  const { ref: menuRef, clampedTop: menuTop } = useClampedMenuTop<HTMLDivElement>(menu?.top ?? 0);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [renameDialogOpen, setRenameDialogOpen] = useState(false);

  useEffect(() => {
    if (!menu) return;
    function close() {
      setMenu(null);
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") close();
    }
    window.addEventListener("click", close);
    window.addEventListener("keydown", onKey);
    // The menu is position: fixed and doesn't follow the row as the list
    // scrolls, so close it on any scroll (standard context-menu behavior).
    // Capture is required because the scroller is a nested element, not window.
    window.addEventListener("scroll", close, { capture: true });
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", close, { capture: true });
    };
  }, [menu]);

  async function handleDelete() {
    setDeleting(true);
    try {
      await deleteAgentSession(session.id);
      setDeleteError(null);
      // Lets the sidebar, menu bar, and App session state drop the row.
      window.setTimeout(() => {
        window.dispatchEvent(
          new CustomEvent(AGENT_DELETE_SESSION_EVENT, {
            detail: { sessionId: session.id },
          }),
        );
      }, 0);
    } catch (err) {
      setDeleteError(messageFromError(err));
      throw err;
    } finally {
      setDeleting(false);
    }
  }

  const rowMain = (
    <>
      {/* The leading icon slot carries the archived state: a scannable
       * left-edge cue in mixed lists without adding a badge. */}
      <span className="folder-note-icon" aria-hidden>
        {archived ? <IconArchive size={15} /> : <IconBubble3 size={15} />}
      </span>
      <span className="folder-note-body">
        <span className="folder-note-title">{title}</span>
        <span className="folder-note-subtitle">{subtitle}</span>
      </span>
    </>
  );

  return (
    <li>
      <div
        className="folder-note-row all-notes-row agent-session-row"
        data-archived={archived || undefined}
        data-selected={!archived && checked}
        data-has-actions="true"
        data-menu-open={menu !== null}
        data-status={status}
      >
        {/* Archived sessions are excluded from bulk selection entirely, so they
         * render no checkbox (an archived row can't be selected, moved, or
         * bulk-deleted — JUN-203 review). */}
        {archived ? null : (
          <label
            className={`folder-note-checkbox${checked ? " folder-note-checkbox-checked" : ""}`}
          >
            <input
              type="checkbox"
              checked={checked}
              aria-label={t("agent.sessions.selectRow", { title })}
              onChange={onToggleSelected}
            />
            <span className="folder-note-select-box" aria-hidden>
              {checked ? <IconCheckmark2Medium size={10} /> : null}
            </span>
          </label>
        )}
        <button type="button" className="folder-note-main" onClick={onSelect}>
          {rowMain}
        </button>
        {status ? (
          <span
            className="agent-session-list-status"
            data-status={status}
            role="status"
            aria-label={statusLabel}
          >
            <span aria-hidden />
            {statusLabel}
          </span>
        ) : (
          <span className="folder-note-time">{formatSessionTime(session.updatedAt)}</span>
        )}
        <span className="folder-note-actions">
          <button
            type="button"
            className="folder-note-menu"
            aria-label={t("agent.sessions.actionsFor", { title })}
            aria-haspopup="menu"
            aria-expanded={menu !== null}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              if (menu) {
                setMenu(null);
                return;
              }
              const rect = event.currentTarget.getBoundingClientRect();
              setMenu({
                right: window.innerWidth - rect.right,
                top: rect.bottom + 4,
              });
            }}
          >
            <IconDotGrid1x3Horizontal size={13} />
          </button>
        </span>
        {menu ? (
          <div
            ref={menuRef}
            className="context-menu"
            style={{ right: menu.right, top: menuTop }}
            role="menu"
            onClick={(event) => event.stopPropagation()}
          >
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setMenu(null);
                setRenameDialogOpen(true);
              }}
            >
              <IconPencil size={14} />
              {t("common.rename")}
            </button>
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setMenu(null);
                onOpenMove();
              }}
            >
              {currentFolderId ? <IconMoveFolder size={14} /> : <IconFolderAddRight size={14} />}
              {currentFolderId
                ? t("agent.sessions.changeProject")
                : t("agent.sessions.addToProject")}
            </button>
            {currentFolderId ? (
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(null);
                  onRemoveFromProject(currentFolderId);
                }}
              >
                <IconFolderDelete size={14} />
                {t("agent.sessions.removeFromProject")}
              </button>
            ) : null}
            {onToggleArchived ? (
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(null);
                  onToggleArchived();
                }}
              >
                {archived ? <IconArrowUndoUp size={14} /> : <IconArchive size={14} />}
                {archived ? t("agent.sessions.unarchive") : t("agent.sessions.archive")}
              </button>
            ) : null}
            <div className="context-menu-separator" role="separator" />
            <button
              type="button"
              role="menuitem"
              className="destructive"
              disabled={deleting}
              onClick={() => {
                setMenu(null);
                setConfirmDelete(true);
              }}
            >
              <IconTrashCan size={14} />
              {t("common.delete")}
            </button>
          </div>
        ) : null}
      </div>
      <RenameSessionDialog
        open={renameDialogOpen}
        currentName={title}
        onClose={() => setRenameDialogOpen(false)}
        onRename={onRename}
      />
      <ConfirmDialog
        open={confirmDelete}
        onClose={() => {
          setConfirmDelete(false);
          setDeleteError(null);
        }}
        onConfirm={() => handleDelete()}
        title={t("agent.sessions.deleteTitle", { title })}
        description={deleteError || t("agent.sessions.deleteDescription")}
        confirmLabel={t("agent.sessions.deleteSession")}
        destructive
      />
    </li>
  );
}

/** Same-day shows the time, within a week the weekday, older the date —
 * matches the note rows so the two lists read identically. */
function formatSessionTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime()) || date.getTime() === 0) return "";
  return formatListTimestamp(date);
}
