import { IconCheckmark2 } from "central-icons-filled/IconCheckmark2";
import { IconCodeAssistant } from "central-icons/IconCodeAssistant";
import { IconProjects } from "central-icons/IconProjects";
import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";
import { IconDotGrid1x3Horizontal } from "central-icons/IconDotGrid1x3Horizontal";
import { IconFolder1 } from "central-icons/IconFolder1";
import { IconFolderAddRight } from "central-icons/IconFolderAddRight";
import { IconFolderDelete } from "central-icons/IconFolderDelete";
import { IconFolderOpen } from "central-icons/IconFolderOpen";
import { IconPencil } from "central-icons/IconPencil";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { IconMoveFolder } from "central-icons/IconMoveFolder";
import { IconNoteText } from "central-icons/IconNoteText";
import { IconPageSearch } from "central-icons/IconPageSearch";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { IconSettingsGear4 } from "central-icons/IconSettingsGear4";
import { IconSortArrowUpDown } from "central-icons/IconSortArrowUpDown";
import { IconTrashCan } from "central-icons/IconTrashCan";
import type { FolderDto, NoteListItemDto } from "../../lib/tauri";
import type { AgentSessionDto } from "../../lib/agent-runtime-contract";
import {
  type DragEvent,
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { formatDate as formatIntlDate, t as translate, useT, useTRich } from "../../i18n";
import { formatListTimestamp, recentAge } from "../../lib/date-format";
import { LEGACY_NOTE_DND_MIME, NOTE_DND_MIME } from "../../lib/dnd";
import { useDismiss } from "../../lib/use-dismiss";
import { useForcedEmptyStates } from "../../lib/empty-states-demo";
import { BreadcrumbBar } from "../ui/BreadcrumbBar";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { EmptyState } from "../ui/EmptyState";
import { AddNotesToFolderDialog } from "./AddNotesToFolderDialog";
import { CreateFolderDialog } from "./CreateFolderDialog";
import { EditFolderDialog } from "./EditFolderDialog";
import { ProjectSettingsDialog } from "./ProjectSettingsDialog";
import { ImportClaudeProjectsDialog } from "./ImportClaudeProjectsDialog";
import { buildFolderItemIndex } from "./folder-item-index";

const NO_FOLDERS: FolderDto[] = [];
const NO_FOLDER_NOTES: readonly NoteListItemDto[] = [];
const NO_FOLDER_SESSIONS: readonly AgentSessionDto[] = [];

type FoldersWorkspaceProps = {
  folders: FolderDto[];
  notes: NoteListItemDto[];
  /** Agent sessions that can be filed into projects alongside notes. */
  /** sessionId -> project (folder) ids the session is filed under. */
  selectedFolderId?: string;
  folderBackTarget?: {
    label: string;
    onBack: () => void;
  };
  onSelectFolder: (folderId?: string) => void;
  onCreateFolder: (name: string, description?: string) => Promise<FolderDto | undefined> | void;
  onFoldersImported?: (folders: FolderDto[]) => void;
  onRenameFolder: (folderId: string, name: string, description?: string) => Promise<unknown> | void;
  onFolderUpdated: (folder: FolderDto) => void;
  onDeleteFolder: (folderId: string, deleteNotes: boolean) => Promise<unknown> | void;
  onCreateNote: (folderId?: string) => void;
  onSelectNote: (noteId: string) => void;
  onAssignNoteToFolder: (noteId: string, folderId: string) => Promise<unknown>;
  onRemoveNoteFromFolder: (noteId: string, folderId: string) => void;
  onOpenMoveDialog: (noteId: string) => void;
  onDeleteNote: (noteId: string) => void;
  /** Open the full Memory manager (Settings > Memory) filtered to a project. */
  onManageProjectMemory: (folderId: string) => void;
};

export function FoldersWorkspace(props: FoldersWorkspaceProps) {
  const { folders, selectedFolderId } = props;
  const folder = useMemo(
    () => folders.find((item) => item.id === selectedFolderId),
    [folders, selectedFolderId],
  );

  if (selectedFolderId && folder) {
    return <FolderDetail {...props} folder={folder} />;
  }
  return <FolderList {...props} />;
}

/* List view -------------------------------------------------------- */

type SortKey = "updated" | "created" | "name" | "nameDesc";

const SORT_OPTIONS: { value: SortKey; readonly label: string }[] = [
  {
    value: "updated",
    get label() {
      return translate("notes.projects.sort.updated");
    },
  },
  {
    value: "created",
    get label() {
      return translate("notes.projects.sort.created");
    },
  },
  {
    value: "name",
    get label() {
      return translate("notes.projects.sort.name");
    },
  },
  {
    value: "nameDesc",
    get label() {
      return translate("notes.projects.sort.nameDesc");
    },
  },
];

function FolderList({
  folders: allFolders,
  notes,
  onSelectFolder,
  onCreateFolder,
  onFoldersImported,
  onRenameFolder,
  onDeleteFolder,
  onAssignNoteToFolder,
}: FoldersWorkspaceProps) {
  const t = useT();
  // __emptyStates() preview (dev console): render the page as a fresh
  // install would see it, real data untouched underneath.
  const folders = useForcedEmptyStates() ? NO_FOLDERS : allFolders;
  const [createOpen, setCreateOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<SortKey>("updated");
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const [editId, setEditId] = useState<string | null>(null);
  const deleteFolderTarget = folders.find((f) => f.id === deleteId);
  const editFolderTarget = folders.find((f) => f.id === editId);

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
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu]);

  const normalizedQuery = query.trim().toLowerCase();

  const sortedAndFiltered = useMemo(() => {
    const filtered = normalizedQuery
      ? folders.filter((folder) =>
          `${folder.name} ${folder.description ?? ""}`.toLowerCase().includes(normalizedQuery),
        )
      : folders;
    return [...filtered].sort((a, b) => {
      switch (sort) {
        case "name":
          return a.name.localeCompare(b.name, undefined, {
            sensitivity: "base",
          });
        case "nameDesc":
          return b.name.localeCompare(a.name, undefined, {
            sensitivity: "base",
          });
        case "created":
          return b.createdAt.localeCompare(a.createdAt);
        case "updated":
        default:
          return b.updatedAt.localeCompare(a.updatedAt);
      }
    });
  }, [folders, normalizedQuery, sort]);
  const folderItems = useMemo(() => buildFolderItemIndex(notes, [], {}), [notes]);

  const content: ReactNode =
    sortedAndFiltered.length > 0 ? (
      <div className="folders-grid" role="list">
        {sortedAndFiltered.map((folder) => (
          <FolderCard
            key={folder.id}
            folder={folder}
            notes={folderItems.notesByFolderId.get(folder.id) ?? NO_FOLDER_NOTES}
            menuOpen={menu?.folderId === folder.id}
            onOpen={() => onSelectFolder(folder.id)}
            onDropNote={(noteId) => {
              const note = notes.find((item) => item.id === noteId);
              if (!note || (note.folderIds.length === 1 && note.folderIds[0] === folder.id)) {
                return;
              }
              void onAssignNoteToFolder(noteId, folder.id);
            }}
            onOpenMenu={(anchor) => {
              if (menu?.folderId === folder.id) {
                setMenu(null);
                return;
              }
              const rect = anchor.getBoundingClientRect();
              setMenu({
                folderId: folder.id,
                right: window.innerWidth - rect.right,
                top: rect.bottom + 4,
              });
            }}
          />
        ))}
      </div>
    ) : folders.length === 0 ? (
      <EmptyState
        label={t("notes.projects.empty.label")}
        icon={<IconFolderOpen size={28} />}
        title={t("notes.projects.empty.title")}
        description={t("notes.projects.empty.description")}
        action={
          <div className="folders-empty-actions">
            <button
              type="button"
              className="primary-action primary-solid"
              onClick={() => setImportOpen(true)}
            >
              <IconCodeAssistant size={14} />
              {t("notes.projects.empty.addClaude")}
            </button>
            <button type="button" className="primary-action" onClick={() => setCreateOpen(true)}>
              <IconFolderAddRight size={14} />
              {t("notes.projects.empty.create")}
            </button>
          </div>
        }
      />
    ) : (
      <div className="folders-empty">
        <p>{t("notes.projects.noMatch", { query: query.trim() })}</p>
      </div>
    );

  return (
    <section className="folders-workspace" aria-label={t("notes.projects.title")}>
      <header className="folders-header">
        <div className="folders-heading">
          <h1>{t("notes.projects.title")}</h1>
          <p className="folders-subtitle">{t("notes.projects.subtitle")}</p>
        </div>
        <div className="folders-header-actions">
          <button type="button" className="primary-action" onClick={() => setImportOpen(true)}>
            <IconCodeAssistant size={14} />
            {t("notes.projects.addExisting")}
          </button>
          <button
            type="button"
            className="primary-action primary-solid folders-create"
            onClick={() => setCreateOpen(true)}
          >
            <IconFolderAddRight size={14} />
            {t("notes.projects.newProject")}
          </button>
        </div>
      </header>

      <div className="folders-controls">
        <label className="folders-search">
          <IconMagnifyingGlass size={14} />
          <input
            type="search"
            aria-label={t("notes.move.searchProjects")}
            placeholder={t("common.search")}
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
          />
        </label>
        <SortDropdown value={sort} onChange={setSort} />
      </div>

      {content}

      {menu ? (
        <FolderCardMenu
          right={menu.right}
          top={menu.top}
          folderId={menu.folderId}
          folders={folders}
          notes={notes}
          onClose={() => setMenu(null)}
          onOpen={(folderId) => {
            onSelectFolder(folderId);
            setMenu(null);
          }}
          onEdit={(folderId) => {
            setEditId(folderId);
            setMenu(null);
          }}
          onRequestDelete={(folderId) => setDeleteId(folderId)}
        />
      ) : null}

      <ConfirmDialog
        open={deleteFolderTarget !== undefined}
        onClose={() => setDeleteId(null)}
        onConfirm={() => {
          if (!deleteFolderTarget) return;
          return onDeleteFolder(deleteFolderTarget.id, false);
        }}
        title={t("notes.projects.delete.title", { name: deleteFolderTarget?.name ?? "" })}
        description={t("notes.projects.delete.description")}
        confirmLabel={t("notes.projectSettings.delete")}
        destructive
      />

      <CreateFolderDialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        onCreate={async (name, description) => {
          await onCreateFolder(name, description);
        }}
      />
      <ImportClaudeProjectsDialog
        open={importOpen}
        onClose={() => setImportOpen(false)}
        onImported={onFoldersImported ?? (() => {})}
      />
      {editFolderTarget ? (
        <EditFolderDialog
          open
          onClose={() => setEditId(null)}
          folder={editFolderTarget}
          onSave={(name, description) => onRenameFolder(editFolderTarget.id, name, description)}
        />
      ) : null}
    </section>
  );
}

function SortDropdown({ value, onChange }: { value: SortKey; onChange: (value: SortKey) => void }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);

  useDismiss(ref, open, () => setOpen(false));

  const current = SORT_OPTIONS.find((option) => option.value === value);

  return (
    <div className="folders-sort" ref={ref}>
      <button
        type="button"
        className="folders-sort-trigger"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((prev) => !prev)}
      >
        <IconSortArrowUpDown size={13} />
        <span>{current?.label ?? t("notes.projects.sort.label")}</span>
        <IconChevronDownSmall size={12} />
      </button>
      {open ? (
        <div className="folders-sort-menu" role="menu">
          {SORT_OPTIONS.map((option) => (
            <button
              key={option.value}
              type="button"
              role="menuitemradio"
              aria-checked={option.value === value}
              className="folders-sort-item"
              onClick={() => {
                onChange(option.value);
                setOpen(false);
              }}
            >
              <span className="folders-sort-check" aria-hidden>
                {option.value === value ? <IconCheckmark2 size={11} /> : null}
              </span>
              {option.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

type MenuState = { folderId: string; right: number; top: number };

function FolderCard({
  folder,
  notes,
  menuOpen,
  onOpen,
  onOpenMenu,
  onDropNote,
}: {
  folder: FolderDto;
  notes: readonly NoteListItemDto[];
  menuOpen: boolean;
  onOpen: () => void;
  onOpenMenu: (anchor: HTMLElement) => void;
  onDropNote: (noteId: string) => void;
}) {
  const t = useT();
  const menuButtonRef = useRef<HTMLButtonElement | null>(null);
  const dragDepth = useRef(0);
  const [dropActive, setDropActive] = useState(false);
  const lastUpdated = notes[0]?.updatedAt ?? folder.updatedAt;

  function hasNoteData(event: DragEvent<HTMLElement>) {
    const types = event.dataTransfer.types;
    for (let i = 0; i < types.length; i += 1) {
      if (types[i] === NOTE_DND_MIME || types[i] === LEGACY_NOTE_DND_MIME) return true;
    }
    return false;
  }

  function resetDropState() {
    dragDepth.current = 0;
    setDropActive(false);
  }

  useEffect(() => {
    if (!dropActive) return;
    document.addEventListener("dragend", resetDropState);
    document.addEventListener("drop", resetDropState);
    return () => {
      document.removeEventListener("dragend", resetDropState);
      document.removeEventListener("drop", resetDropState);
    };
  }, [dropActive]);

  return (
    <article
      className="folder-card"
      data-menu-open={menuOpen}
      data-drop-active={dropActive || undefined}
      role="button"
      tabIndex={0}
      aria-label={t("notes.folderChip.open", { name: folder.name })}
      onClick={onOpen}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onOpen();
        }
      }}
      onContextMenu={(event) => {
        event.preventDefault();
        if (menuButtonRef.current) onOpenMenu(menuButtonRef.current);
      }}
      onDragEnter={(event) => {
        if (!hasNoteData(event)) return;
        event.preventDefault();
        dragDepth.current += 1;
        setDropActive(true);
      }}
      onDragOver={(event) => {
        if (!hasNoteData(event)) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "link";
      }}
      onDragLeave={(event) => {
        if (!hasNoteData(event)) return;
        dragDepth.current = Math.max(0, dragDepth.current - 1);
        if (dragDepth.current === 0) resetDropState();
      }}
      onDrop={(event) => {
        if (!hasNoteData(event)) return;
        event.preventDefault();
        const noteId =
          event.dataTransfer.getData(NOTE_DND_MIME) ||
          event.dataTransfer.getData(LEGACY_NOTE_DND_MIME);
        resetDropState();
        if (noteId) onDropNote(noteId);
      }}
    >
      <div className="folder-card-icon" aria-hidden>
        <IconFolder1 size={13} />
      </div>
      <div className="folder-card-body">
        <div className="folder-card-text">
          <h3 className="folder-card-title">{folder.name}</h3>
          {folder.description ? <p className="folder-card-meta">{folder.description}</p> : null}
          {folder.localPath ? (
            <p className="folder-card-path" title={folder.localPath}>
              {folder.localPath}
            </p>
          ) : null}
        </div>
        <p className="folder-card-footer">
          <span className="folder-card-footer-item">
            <span className="folder-card-footer-icon" aria-hidden>
              <IconNoteText size={11} />
            </span>
            {t("notes.projects.noteCount", { count: notes.length })}
          </span>
          <span className="metadata-dot" aria-hidden />
          <span>{t("notes.projects.updated", { when: formatRelative(lastUpdated) })}</span>
        </p>
      </div>
      <button
        ref={menuButtonRef}
        type="button"
        className="folder-card-menu"
        aria-label={t("notes.list.actionsFor", { title: folder.name })}
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          onOpenMenu(event.currentTarget);
        }}
      >
        <IconDotGrid1x3Horizontal size={14} />
      </button>
    </article>
  );
}

function FolderCardMenu({
  right,
  top,
  folderId,
  folders,
  onClose,
  onOpen,
  onEdit,
  onRequestDelete,
}: {
  right: number;
  top: number;
  folderId: string;
  folders: FolderDto[];
  notes: NoteListItemDto[];
  onClose: () => void;
  onOpen: (folderId: string) => void;
  onEdit: (folderId: string) => void;
  onRequestDelete: (folderId: string) => void;
}) {
  const t = useT();
  const folder = folders.find((item) => item.id === folderId);
  if (!folder) return null;

  return (
    <div
      className="context-menu"
      style={{ right, top }}
      role="menu"
      onClick={(event) => event.stopPropagation()}
    >
      <button type="button" role="menuitem" onClick={() => onOpen(folder.id)}>
        <IconFolderOpen size={14} />
        {t("common.open")}
      </button>
      <button type="button" role="menuitem" onClick={() => onEdit(folder.id)}>
        <IconPencil size={14} />
        {t("notes.projects.editDetails")}
      </button>
      <button
        type="button"
        role="menuitem"
        className="destructive"
        onClick={() => {
          onClose();
          onRequestDelete(folder.id);
        }}
      >
        <IconTrashCan size={14} />
        {t("common.delete")}
      </button>
    </div>
  );
}

/* Detail view ------------------------------------------------------ */

function FolderDetail({
  folder,
  folderBackTarget,
  folders,
  notes,
  onSelectFolder,
  onRenameFolder,
  onFolderUpdated,
  onDeleteFolder,
  onCreateNote,
  onSelectNote,
  onAssignNoteToFolder,
  onRemoveNoteFromFolder,
  onOpenMoveDialog,
  onDeleteNote,
  onManageProjectMemory,
}: FoldersWorkspaceProps & { folder: FolderDto }) {
  const t = useT();
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState(folder.name);
  const [menu, setMenu] = useState<{ right: number; top: number } | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const titleRef = useRef<HTMLInputElement | null>(null);

  useLayoutEffect(() => {
    if (editingTitle && titleRef.current) {
      titleRef.current.focus();
      titleRef.current.select();
    }
  }, [editingTitle]);

  useEffect(() => {
    if (!editingTitle) setTitleDraft(folder.name);
  }, [folder.name, editingTitle]);

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
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu]);

  // __emptyStates() preview (dev console): an open project renders as if it
  // held nothing, so the folder empty state is reachable too.
  const forcedEmpty = useForcedEmptyStates();

  const folderNotes = useMemo(
    () =>
      forcedEmpty
        ? []
        : notes
            .filter((note) => note.folderIds.includes(folder.id))
            .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)),
    [notes, folder.id, forcedEmpty],
  );

  const lastUpdated = folderNotes[0]?.updatedAt ?? folder.updatedAt;

  function commitRename() {
    const next = titleDraft.trim();
    setEditingTitle(false);
    if (!next || next === folder.name) {
      setTitleDraft(folder.name);
      return;
    }
    onRenameFolder(folder.id, next, folder.description ?? undefined);
  }

  function openMenu(anchor: HTMLElement) {
    const rect = anchor.getBoundingClientRect();
    setMenu({
      right: window.innerWidth - rect.right,
      top: rect.bottom + 4,
    });
  }

  return (
    <section className="folder-detail" aria-label={folder.name}>
      <BreadcrumbBar
        backLabel={folderBackTarget?.label ?? t("notes.projects.back")}
        onBack={folderBackTarget?.onBack ?? (() => onSelectFolder(undefined))}
        items={[
          { label: t("notes.projects.title"), onClick: () => onSelectFolder(undefined) },
          { label: folder.name, icon: <IconProjects size={13} /> },
        ]}
        actions={
          <button
            type="button"
            className="ghost-icon-button"
            aria-label={t("notes.list.actionsFor", { title: folder.name })}
            aria-haspopup="menu"
            aria-expanded={menu !== null}
            onClick={(event) => {
              event.stopPropagation();
              if (menu) {
                setMenu(null);
                return;
              }
              openMenu(event.currentTarget);
            }}
          >
            <IconDotGrid1x3Horizontal size={14} />
          </button>
        }
      />

      <div className="folder-detail-content">
        <header className="folder-detail-header">
          <FolderAddMenu
            onCreateNote={() => onCreateNote(folder.id)}
            onAddExisting={() => setAddOpen(true)}
            hasNotesElsewhere={notes.some((note) => !note.folderIds.includes(folder.id))}
          />
          {editingTitle ? (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                commitRename();
              }}
            >
              <input
                ref={titleRef}
                className="folder-detail-title-input"
                value={titleDraft}
                onChange={(event) => setTitleDraft(event.currentTarget.value)}
                onBlur={commitRename}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    event.preventDefault();
                    setTitleDraft(folder.name);
                    setEditingTitle(false);
                  }
                }}
              />
            </form>
          ) : (
            <h1
              className="folder-detail-title"
              tabIndex={0}
              role="button"
              aria-label={t("notes.projects.rename")}
              onClick={() => setEditingTitle(true)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  setEditingTitle(true);
                }
              }}
            >
              {folder.name}
            </h1>
          )}
          {folder.description ? (
            <p className="folder-detail-description">{folder.description}</p>
          ) : null}
          {folder.localPath ? (
            <p className="folder-detail-path" title={folder.localPath}>
              <IconFolder1 size={12} aria-hidden />
              <span>{folder.localPath}</span>
            </p>
          ) : null}
          <p className="folder-detail-meta">
            <span className="folder-detail-meta-pill" aria-hidden>
              <IconNoteText size={12} />
            </span>
            {t("notes.projects.noteCount", { count: folderNotes.length })}
            <span className="metadata-dot" aria-hidden />
            {t("notes.projects.updated", { when: formatDate(lastUpdated) })}
          </p>
        </header>

        {folderNotes.length > 0 ? (
          <>
            <div className="folder-actions-row">
              <h2 className="folder-notes-title">{t("notes.list.title")}</h2>
            </div>
            <FolderNoteList
              folder={folder}
              notes={folderNotes}
              onSelectNote={onSelectNote}
              onOpenMoveDialog={onOpenMoveDialog}
              onRemoveNoteFromFolder={onRemoveNoteFromFolder}
              onDeleteNote={onDeleteNote}
            />
          </>
        ) : (
          <FolderEmptyState
            onCreateNote={() => onCreateNote(folder.id)}
            onOpenSettings={() => setSettingsOpen(true)}
          />
        )}
      </div>

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
              setMenu(null);
              setSettingsOpen(true);
            }}
          >
            <IconSettingsGear4 size={14} />
            {t("notes.projectSettings.title")}
          </button>
          <button
            type="button"
            role="menuitem"
            className="destructive"
            onClick={() => {
              setMenu(null);
              setDeleteOpen(true);
            }}
          >
            <IconTrashCan size={14} />
            {t("notes.projectSettings.delete")}
          </button>
        </div>
      ) : null}

      <AddNotesToFolderDialog
        open={addOpen}
        onClose={() => setAddOpen(false)}
        folder={folder}
        notes={notes}
        onAdd={async (noteId) => {
          await onAssignNoteToFolder(noteId, folder.id);
        }}
      />
      <ConfirmDialog
        open={deleteOpen}
        onClose={() => setDeleteOpen(false)}
        onConfirm={() => onDeleteFolder(folder.id, false)}
        title={t("notes.projects.delete.title", { name: folder.name })}
        description={t("notes.projects.delete.description")}
        confirmLabel={t("notes.projectSettings.delete")}
        destructive
      />
      <ProjectSettingsDialog
        open={settingsOpen}
        folder={folder}
        onClose={() => setSettingsOpen(false)}
        onSaveDetails={(name, description) => onRenameFolder(folder.id, name, description)}
        onFolderUpdated={onFolderUpdated}
        onManageMemory={onManageProjectMemory}
        onRequestDelete={() => setDeleteOpen(true)}
      />
    </section>
  );
}

function FolderNoteList({
  folder,
  notes,
  onSelectNote,
  onOpenMoveDialog,
  onRemoveNoteFromFolder,
  onDeleteNote,
}: {
  folder: FolderDto;
  notes: NoteListItemDto[];
  onSelectNote: (noteId: string) => void;
  onOpenMoveDialog: (noteId: string) => void;
  onRemoveNoteFromFolder: (noteId: string, folderId: string) => void;
  onDeleteNote: (noteId: string) => void;
}) {
  return (
    <ul className="folder-notes" role="list">
      {notes.map((note) => (
        <FolderNoteRow
          key={note.id}
          note={note}
          folder={folder}
          onSelect={() => onSelectNote(note.id)}
          onOpenMove={() => onOpenMoveDialog(note.id)}
          onRemoveFromFolder={() => onRemoveNoteFromFolder(note.id, folder.id)}
          onDelete={() => onDeleteNote(note.id)}
        />
      ))}
    </ul>
  );
}

/** Single "+" up in the project header — one menu for everything that can
 * land in the project. */
function FolderAddMenu({
  onCreateNote,
  onAddExisting,
  hasNotesElsewhere,
}: {
  onCreateNote: () => void;
  onAddExisting: () => void;
  hasNotesElsewhere: boolean;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!open) return;
    function close() {
      setOpen(false);
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") close();
    }
    window.addEventListener("click", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div className="folder-detail-add">
      <button
        type="button"
        className="folder-add-trigger"
        aria-label={t("notes.projects.addToProject")}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={(event) => {
          event.stopPropagation();
          setOpen((value) => !value);
        }}
      >
        <IconPlusMedium size={15} />
      </button>
      {open ? (
        <div
          className="folder-add-popover"
          role="menu"
          onClick={(event) => event.stopPropagation()}
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              onCreateNote();
            }}
          >
            <IconNoteText size={14} />
            {t("notes.projects.newMeetingNote")}
          </button>
          {hasNotesElsewhere ? <div className="context-menu-separator" role="separator" /> : null}
          {hasNotesElsewhere ? (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setOpen(false);
                onAddExisting();
              }}
            >
              <IconPageSearch size={14} />
              {t("notes.projects.addExistingNote")}
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function FolderNoteRow({
  note,
  folder: _folder,
  onSelect,
  onOpenMove,
  onRemoveFromFolder,
  onDelete,
}: {
  note: NoteListItemDto;
  folder: FolderDto;
  onSelect: () => void;
  onOpenMove: () => void;
  onRemoveFromFolder: () => void;
  onDelete: () => void;
}) {
  const t = useT();
  const [menu, setMenu] = useState<{ right: number; top: number } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

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
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu]);

  return (
    <li>
      <div className="folder-note-row" data-has-actions="true" data-menu-open={menu !== null}>
        <button type="button" className="folder-note-main" onClick={onSelect}>
          <span className="folder-note-icon" aria-hidden>
            <IconNoteText size={14} />
          </span>
          <span className="folder-note-body">
            <span className="folder-note-title">
              {note.title.trim() || t("notes.editor.titlePlaceholder")}
            </span>
            <span className="folder-note-subtitle">
              {note.preview.trim()
                ? note.preview
                : t("notes.projects.updated", { when: formatRelative(note.updatedAt) })}
            </span>
          </span>
        </button>
        <span className="folder-note-time">{formatNoteTime(note.updatedAt)}</span>
        <span className="folder-note-actions">
          <button
            type="button"
            className="folder-note-menu"
            aria-label={t("notes.list.actionsFor", {
              title: note.title.trim() || t("notes.projects.thisMeetingNote"),
            })}
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
            className="context-menu"
            style={{ right: menu.right, top: menu.top }}
            role="menu"
            onClick={(event) => event.stopPropagation()}
          >
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setMenu(null);
                onOpenMove();
              }}
            >
              <IconMoveFolder size={14} />
              {t("notes.projects.changeProject")}
            </button>
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setMenu(null);
                onRemoveFromFolder();
              }}
            >
              <IconFolderDelete size={14} />
              {t("notes.projects.removeFromProject")}
            </button>
            <div className="context-menu-separator" role="separator" />
            <button
              type="button"
              role="menuitem"
              className="destructive"
              onClick={() => {
                setMenu(null);
                setConfirmDelete(true);
              }}
            >
              <IconTrashCan size={14} />
              {t("notes.projects.deleteMeetingNote")}
            </button>
          </div>
        ) : null}
      </div>
      <ConfirmDialog
        open={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        onConfirm={onDelete}
        title={t("notes.list.delete.title", {
          title: note.title.trim() || t("notes.editor.titlePlaceholder"),
        })}
        description={t("notes.list.delete.description")}
        confirmLabel={t("notes.projects.deleteMeetingNote")}
        destructive
      />
    </li>
  );
}

function FolderEmptyState({
  onCreateNote,
  onOpenSettings,
}: {
  onCreateNote: () => void;
  onOpenSettings: () => void;
}) {
  const t = useT();
  const tr = useTRich();
  return (
    <EmptyState
      label={t("notes.projects.detailEmpty.label")}
      icon={<IconFolderOpen size={28} />}
      title={t("notes.projects.detailEmpty.title")}
      description={t("notes.projects.detailEmpty.description")}
      action={
        <>
          <div className="folder-empty-cta">
            <button type="button" className="primary-action primary-solid" onClick={onCreateNote}>
              <IconPlusMedium size={13} />
              {t("notes.projects.newMeetingNote")}
            </button>
          </div>
          <p className="folder-empty-nudge">
            {tr("notes.projects.detailEmpty.nudge", {
              settings: (chunks) => (
                <button
                  type="button"
                  className="settings-inline-link folder-empty-nudge-link"
                  onClick={onOpenSettings}
                >
                  {chunks}
                </button>
              ),
            })}
          </p>
        </>
      }
    />
  );
}

/* Formatting helpers ----------------------------------------------- */

const RELATIVE_AGE_KEYS = {
  now: "notes.projects.relative.justNow",
  minutes: "notes.projects.relative.minutes",
  hours: "notes.projects.relative.hours",
  days: "notes.projects.relative.days",
} as const;

function formatRelative(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const age = recentAge(date);
  if (age) return translate(RELATIVE_AGE_KEYS[age.unit], { count: age.count });
  return formatIntlDate(date, {
    month: "short",
    day: "numeric",
  });
}

function formatDate(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const now = new Date();
  const sameYear = date.getFullYear() === now.getFullYear();
  return formatIntlDate(date, {
    month: "short",
    day: "numeric",
    year: sameYear ? undefined : "numeric",
  });
}

/** Right-aligned timestamp inside a folder's note row. Same-day shows
 * just the time ("2:09 PM"). Within a week, the weekday ("Mon"). Older,
 * the date ("May 22"). */
function formatNoteTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return formatListTimestamp(date);
}
