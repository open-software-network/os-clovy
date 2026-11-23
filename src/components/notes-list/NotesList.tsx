import { IconCheckmark2Medium } from "central-icons-filled/IconCheckmark2Medium";
import { IconCrossMedium } from "central-icons/IconCrossMedium";
import { IconDotGrid1x3Horizontal } from "central-icons/IconDotGrid1x3Horizontal";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { IconMoveFolder } from "central-icons/IconMoveFolder";
import { IconNoteText } from "central-icons/IconNoteText";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { IconTrashCan } from "central-icons/IconTrashCan";
import {
  forwardRef,
  memo,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";
import { t as translate, useT } from "../../i18n";
import { formatListTimestamp } from "../../lib/date-format";
import type { NoteListItemDto } from "../../lib/tauri";
import { useDismiss } from "../../lib/use-dismiss";
import { useForcedEmptyStates } from "../../lib/empty-states-demo";
import { primaryShiftShortcutLabel } from "../../lib/platform";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { EmptyState } from "../ui/EmptyState";
import { filterNotesByQuery } from "./notes-list-helpers";

const NO_NOTES: NoteListItemDto[] = [];

type NotesListProps = {
  notes: NoteListItemDto[];
  activeRecordingNoteId?: string;
  onSelectNote: (noteId: string) => void;
  onCreateNote: () => void;
  onOpenMoveDialog: (noteId: string) => void;
  onOpenMoveNotes: (noteIds: string[]) => void;
  onDeleteNote: (noteId: string) => void;
  onDeleteNotes: (noteIds: string[]) => void | Promise<unknown>;
};

export type NotesListHandle = {
  resetSelection: () => void;
};

type MenuPosition = {
  right: number;
  top: number;
};

type OpenMenu = MenuPosition & {
  noteId: string;
};

const CONTEXT_MENU_WIDTH = 156;
const MEETING_MENU_HEIGHT = 74;
const MENU_GAP = 4;
const VIEWPORT_MARGIN = 8;

export const NotesList = forwardRef<NotesListHandle, NotesListProps>(function NotesList(
  {
    notes: allNotes,
    activeRecordingNoteId,
    onSelectNote,
    onCreateNote,
    onOpenMoveDialog,
    onOpenMoveNotes,
    onDeleteNote,
    onDeleteNotes,
  },
  ref,
) {
  const t = useT();
  // __emptyStates() preview (dev console): render the page as a fresh
  // install would see it, real data untouched underneath.
  const notes = useForcedEmptyStates() ? NO_NOTES : allNotes;
  const [query, setQuery] = useState("");
  const [openMenu, setOpenMenu] = useState<OpenMenu | null>(null);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [confirmBulkDelete, setConfirmBulkDelete] = useState(false);
  // The bar plays one of two exits when the selection ends. A deliberate
  // dismiss (×, Escape, post-delete) slides out; the selection merely emptying
  // (deselect all, unchecking the last row) fades. We capture the cause the
  // moment the selection drops to zero, keep the bar mounted with data-exit,
  // and unmount when its animation finishes.
  const [exit, setExit] = useState<null | "slide" | "fade">(null);
  const createNoteShortcut = primaryShiftShortcutLabel("N");
  // The cause of the *next* empty transition, latched by the call sites.
  // Toggling a row can't know it's the last box until the set settles, so
  // unchecking defaults to fade unless a dismiss intent was latched first.
  const exitCauseRef = useRef<"slide" | "fade">("fade");

  const sortedNotes = useMemo(
    () => [...notes].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)),
    [notes],
  );
  const filteredNotes = useMemo(() => filterNotesByQuery(sortedNotes, query), [sortedNotes, query]);

  const selectedNoteIds = useMemo(
    () => sortedNotes.filter((note) => selectedIds.has(note.id)).map((note) => note.id),
    [sortedNotes, selectedIds],
  );
  const visibleSelectedCount = filteredNotes.filter((note) => selectedIds.has(note.id)).length;
  const selectedCount = selectedNoteIds.length;
  const hasUnselectedVisibleNotes =
    filteredNotes.length > 0 && visibleSelectedCount < filteredNotes.length;

  // Snapshot the last nonzero count so the exiting bar keeps its real label
  // instead of flashing "0 selected".
  const lastCountRef = useRef(selectedCount);
  if (selectedCount > 0) lastCountRef.current = selectedCount;
  const displayCount = selectedCount > 0 ? selectedCount : lastCountRef.current;

  // Drive the exit/cancel transitions off the live count. Selecting again
  // mid-exit cancels the exit and replays the live bar. The previous-count
  // guard keeps the initial mount (0, with nothing to exit from) from arming
  // a ghost exit.
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

  useEffect(() => {
    const noteIds = new Set(notes.map((note) => note.id));
    setSelectedIds((previous) => {
      const next = new Set([...previous].filter((id) => noteIds.has(id)));
      return next.size === previous.size ? previous : next;
    });
    if (notes.length === 0) {
      setConfirmBulkDelete(false);
    }
  }, [notes]);

  const toggleSelected = useCallback((noteId: string) => {
    setSelectedIds((previous) => {
      const next = new Set(previous);
      if (next.has(noteId)) next.delete(noteId);
      else next.add(noteId);
      return next;
    });
  }, []);

  const selectNote = useCallback((noteId: string) => onSelectNote(noteId), [onSelectNote]);
  const openNoteMenu = useCallback((noteId: string, position: MenuPosition) => {
    setOpenMenu({ noteId, ...position });
  }, []);
  const closeNoteMenu = useCallback(() => setOpenMenu(null), []);
  const openMoveDialog = useCallback(
    (noteId: string) => onOpenMoveDialog(noteId),
    [onOpenMoveDialog],
  );
  const deleteNote = useCallback((noteId: string) => onDeleteNote(noteId), [onDeleteNote]);

  // A deliberate dismiss — ×, Escape, post-delete — slides the bar out.
  const resetSelection = useCallback(() => {
    exitCauseRef.current = "slide";
    setSelectedIds(new Set());
    setConfirmBulkDelete(false);
  }, []);

  useImperativeHandle(ref, () => ({ resetSelection }), [resetSelection]);

  function selectAllVisibleNotes() {
    setSelectedIds((previous) => {
      const next = new Set(previous);
      for (const note of filteredNotes) {
        next.add(note.id);
      }
      return next;
    });
  }

  function deselectAllVisibleNotes() {
    setSelectedIds((previous) => {
      const next = new Set(previous);
      for (const note of filteredNotes) {
        next.delete(note.id);
      }
      return next;
    });
  }

  // Escape clears an active selection. Skipped while the bulk-delete confirm
  // dialog is open so the two don't fight over the same keypress — there the
  // dialog owns Escape (it dismisses itself, leaving the selection intact).
  useEffect(() => {
    if (selectedCount === 0 || confirmBulkDelete) return;
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") resetSelection();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedCount, confirmBulkDelete]);

  return (
    <section className="all-notes-workspace" aria-label={t("notes.list.title")}>
      <header className="folders-header">
        <div className="folders-heading">
          <h1>
            {t("notes.list.title")}
            {notes.length > 0 ? <span className="folders-count">{notes.length}</span> : null}
          </h1>
          <p className="folders-subtitle">{t("notes.list.subtitle")}</p>
        </div>
        <button
          type="button"
          className="primary-action primary-solid folders-create"
          onClick={onCreateNote}
        >
          <IconPlusMedium size={13} />
          {t("notes.list.newNote")}
          <kbd className="primary-action-kbd" aria-hidden>
            {createNoteShortcut}
          </kbd>
        </button>
      </header>

      {notes.length > 0 ? (
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
        </div>
      ) : null}

      {notes.length === 0 ? (
        <EmptyState
          label={t("notes.list.empty.cta")}
          icon={<IconNoteText size={28} />}
          title={t("notes.list.empty.title")}
          description={t("notes.list.empty.description")}
          action={
            <button type="button" className="primary-action primary-solid" onClick={onCreateNote}>
              <IconPlusMedium size={13} />
              {t("notes.list.empty.cta")}
            </button>
          }
        />
      ) : filteredNotes.length === 0 ? (
        <div className="folders-empty">
          <p>{t("notes.list.noMatch", { query: query.trim() })}</p>
        </div>
      ) : (
        <ul className="folder-notes all-notes-list" role="list" data-selecting={selectedCount > 0}>
          {filteredNotes.map((note) => (
            <AllNoteRow
              key={note.id}
              note={note}
              liveRecording={note.id === activeRecordingNoteId}
              menu={openMenu?.noteId === note.id ? openMenu : null}
              onSelect={selectNote}
              checked={selectedIds.has(note.id)}
              onToggleSelected={toggleSelected}
              onOpenMenu={openNoteMenu}
              onCloseMenu={closeNoteMenu}
              onOpenMove={openMoveDialog}
              onDelete={deleteNote}
            />
          ))}
        </ul>
      )}

      {barMounted ? (
        <div
          className="meetings-bulk-bar"
          role="toolbar"
          aria-label={t("notes.list.selection")}
          data-exit={isExiting ? exit : undefined}
          onAnimationEnd={(event) => {
            // Only the bar's own exit keyframes unmount it. Child button
            // hovers fire transitionend, not animationend, but a descendant
            // animation could still bubble here — so require the event to
            // originate on the bar and, when the name is reported, to be an
            // exit keyframe. (jsdom omits animationName, hence the optional
            // check rather than a hard requirement.)
            if (!isExiting || event.target !== event.currentTarget) return;
            if (event.animationName && !event.animationName.startsWith("meetings-bulk-bar-out")) {
              return;
            }
            setExit(null);
          }}
        >
          <span className="meetings-bulk-count">
            {t("notes.list.selectedCount", { count: displayCount })}
          </span>
          {hasUnselectedVisibleNotes ? (
            <button type="button" className="meetings-bulk-action" onClick={selectAllVisibleNotes}>
              {t("notes.list.selectAll")}
            </button>
          ) : null}
          <button type="button" className="meetings-bulk-action" onClick={deselectAllVisibleNotes}>
            {t("notes.list.deselectAll")}
          </button>
          <button
            type="button"
            className="meetings-bulk-action"
            onClick={() => onOpenMoveNotes(selectedNoteIds)}
          >
            {t("notes.list.move")}
          </button>
          <button
            type="button"
            className="meetings-bulk-action"
            onClick={() => setConfirmBulkDelete(true)}
          >
            {t("common.delete")}
          </button>
          <button
            type="button"
            className="meetings-bulk-dismiss"
            aria-label={t("notes.list.clearSelection")}
            onClick={resetSelection}
          >
            <IconCrossMedium size={14} />
          </button>
        </div>
      ) : null}

      <ConfirmDialog
        open={confirmBulkDelete}
        onClose={() => setConfirmBulkDelete(false)}
        onConfirm={async () => {
          await onDeleteNotes(selectedNoteIds);
          resetSelection();
        }}
        title={t("notes.list.bulkDelete.title", { count: selectedCount })}
        description={t("notes.list.bulkDelete.description")}
        confirmLabel={t("notes.list.bulkDelete.confirm", { count: selectedCount })}
        destructive
      />
    </section>
  );
});

const AllNoteRow = memo(function AllNoteRow({
  note,
  liveRecording,
  menu,
  onSelect,
  checked,
  onToggleSelected,
  onOpenMenu,
  onCloseMenu,
  onOpenMove,
  onDelete,
}: {
  note: NoteListItemDto;
  liveRecording: boolean;
  menu: MenuPosition | null;
  onSelect: (noteId: string) => void;
  checked: boolean;
  onToggleSelected: (noteId: string) => void;
  onOpenMenu: (noteId: string, position: MenuPosition) => void;
  onCloseMenu: () => void;
  onOpenMove: (noteId: string) => void;
  onDelete: (noteId: string) => void;
}) {
  const t = useT();
  const title = note.title.trim() || t("notes.editor.titlePlaceholder");
  const effectiveStatus =
    note.processingStatus === "recording" && !liveRecording ? "draft" : note.processingStatus;
  const preview =
    note.preview.trim() ||
    (liveRecording ? t("notes.list.status.recording") : statusLabel(effectiveStatus));
  const [confirmDelete, setConfirmDelete] = useState(false);

  useDismiss(null, menu !== null, onCloseMenu, { pointerEvent: "click" });

  return (
    <li>
      <div
        className="folder-note-row all-notes-row"
        data-selected={checked}
        data-has-actions="true"
        data-menu-open={menu !== null}
      >
        <label className={`folder-note-checkbox${checked ? " folder-note-checkbox-checked" : ""}`}>
          <input
            type="checkbox"
            checked={checked}
            aria-label={t("notes.list.selectNote", { title })}
            onChange={() => onToggleSelected(note.id)}
          />
          <span className="folder-note-select-box" aria-hidden>
            {checked ? <IconCheckmark2Medium size={10} /> : null}
          </span>
        </label>
        <button type="button" className="folder-note-main" onClick={() => onSelect(note.id)}>
          <MeetingRowContent
            title={title}
            preview={preview}
            status={effectiveStatus}
            liveRecording={liveRecording}
            showingStatus={!note.preview.trim()}
          />
        </button>
        <span className="folder-note-time">{formatNoteTime(note.updatedAt)}</span>
        <span className="folder-note-actions">
          <button
            type="button"
            className="folder-note-menu"
            aria-label={t("notes.list.actionsFor", { title })}
            aria-haspopup="menu"
            aria-expanded={menu !== null}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              if (menu) {
                onCloseMenu();
                return;
              }
              onOpenMenu(note.id, meetingMenuPosition(event.currentTarget));
            }}
          >
            <IconDotGrid1x3Horizontal size={13} />
          </button>
        </span>
        {menu ? (
          <div
            className="context-menu"
            style={{ right: menu.right, top: menu.top }}
            role="menu"
            onClick={(event) => event.stopPropagation()}
          >
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                onCloseMenu();
                onOpenMove(note.id);
              }}
            >
              <IconMoveFolder size={14} />
              {t("notes.list.moveToProject")}
            </button>
            <div className="context-menu-separator" role="separator" />
            <button
              type="button"
              role="menuitem"
              className="destructive"
              onClick={() => {
                onCloseMenu();
                setConfirmDelete(true);
              }}
            >
              <IconTrashCan size={14} />
              {t("notes.header.deleteNote")}
            </button>
          </div>
        ) : null}
      </div>
      <ConfirmDialog
        open={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        onConfirm={() => onDelete(note.id)}
        title={t("notes.list.delete.title", { title })}
        description={t("notes.list.delete.description")}
        confirmLabel={t("notes.header.deleteNote")}
        destructive
      />
    </li>
  );
});

function MeetingRowContent({
  title,
  preview,
  status,
  liveRecording,
  showingStatus,
}: {
  title: string;
  preview: string;
  status?: NoteListItemDto["processingStatus"];
  liveRecording?: boolean;
  // The subtitle is the bare status label (no real preview yet), so it's worth
  // animating as the live status rather than leaving it as static text.
  showingStatus?: boolean;
}) {
  // The work-in-flight states: the status word shimmers to read as "running",
  // the way the agent rows signal an active session.
  const processing =
    showingStatus &&
    (status === "transcribing" || status === "generating" || status === "validating");
  return (
    <>
      <span className="folder-note-icon" aria-hidden>
        <IconNoteText size={14} />
      </span>
      <span className="folder-note-body">
        <span className="folder-note-title">{title}</span>
        <span className="folder-note-subtitle">
          {liveRecording ? <span className="note-recording-dot" aria-hidden /> : null}
          <span
            data-shimmer={processing ? "true" : undefined}
            className={processing ? "shimmer" : undefined}
          >
            {preview}
          </span>
        </span>
      </span>
    </>
  );
}

function meetingMenuPosition(trigger: HTMLElement): MenuPosition {
  const rect = trigger.getBoundingClientRect();
  const maxRight = Math.max(
    VIEWPORT_MARGIN,
    window.innerWidth - CONTEXT_MENU_WIDTH - VIEWPORT_MARGIN,
  );
  const right = Math.min(
    Math.max(window.innerWidth - rect.left + MENU_GAP, VIEWPORT_MARGIN),
    maxRight,
  );
  const belowTop = rect.bottom + MENU_GAP;
  const top =
    belowTop + MEETING_MENU_HEIGHT <= window.innerHeight - VIEWPORT_MARGIN
      ? belowTop
      : rect.top - MEETING_MENU_HEIGHT - MENU_GAP;

  return {
    right,
    top: Math.max(VIEWPORT_MARGIN, top),
  };
}

function statusLabel(status: NoteListItemDto["processingStatus"]) {
  switch (status) {
    case "recording":
      return translate("notes.list.status.recording");
    case "validating":
      return translate("notes.list.status.validating");
    case "transcribing":
      return translate("notes.list.status.transcribing");
    case "generating":
      return translate("notes.list.status.generating");
    case "failed":
      return translate("notes.list.status.failed");
    case "recoverable":
      return translate("notes.list.status.recoverable");
    case "ready":
      return translate("notes.list.status.ready");
    default:
      return translate("notes.list.status.draft");
  }
}

function formatNoteTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const now = new Date();
  if (now.getTime() < date.getTime()) return translate("notes.list.future");
  return formatListTimestamp(date, now);
}
