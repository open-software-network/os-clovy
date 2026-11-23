import { IconCheckmark2 } from "central-icons-filled/IconCheckmark2";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconProjects } from "central-icons/IconProjects";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { useEffect, useMemo, useRef, useState } from "react";
import type { FolderDto } from "../../lib/tauri";
import type { AgentSessionDto } from "../../lib/agent-runtime-contract";
import { useT } from "../../i18n";
import { Dialog } from "../ui/Dialog";

type Props = {
  open: boolean;
  onClose: () => void;
  sessions: AgentSessionDto[];
  /** sessionId -> project ids the session is currently filed under. */
  sessionFolderIds: Record<string, string[]>;
  folders: FolderDto[];
  onSetFolder: (sessionId: string, folderId: string) => Promise<unknown> | void;
  /**
   * Creates a project from the search query so the session can be filed
   * without leaving the dialog. Same creation path the Projects view uses;
   * resolving to undefined means creation failed (the caller surfaces the
   * error).
   */
  onCreateFolder?: (name: string) => Promise<FolderDto | undefined> | FolderDto | undefined;
  /**
   * Unfiles the session from its current project. When provided, the current
   * project stays in the list with a checkmark and clicking it removes -
   * the same toggle the note editor's project chip uses.
   */
  onRemoveFolder?: (sessionId: string, folderId: string) => Promise<unknown> | void;
  onMoved?: () => void;
};

export function MoveSessionToProjectDialog({
  open,
  onClose,
  sessions,
  sessionFolderIds,
  folders,
  onSetFolder,
  onCreateFolder,
  onRemoveFolder,
  onMoved,
}: Props) {
  const t = useT();
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSelectedId(null);
    setSubmitting(false);
  }, [open]);

  const isSingle = sessions.length === 1;
  const sharedFolderId =
    sessions.length > 0 &&
    sessions.every(
      (session) => sessionFolderIds[session.id]?.[0] === sessionFolderIds[sessions[0].id]?.[0],
    )
      ? sessionFolderIds[sessions[0].id]?.[0]
      : undefined;
  const currentFolderId = sharedFolderId;
  const currentFolder = folders.find((f) => f.id === currentFolderId);
  const hasCurrent = isSingle && Boolean(currentFolder);

  // With onRemoveFolder wired, the current project stays listed (checked) so
  // clicking it can unfile; without it, it's excluded as before.
  const includeCurrent = Boolean(onRemoveFolder) && Boolean(currentFolder);
  const candidates = useMemo(() => {
    const available = includeCurrent
      ? folders
      : folders.filter((folder) => folder.id !== currentFolderId);
    const normalized = query.trim().toLowerCase();
    const filtered = normalized
      ? available.filter((folder) =>
          `${folder.name} ${folder.description ?? ""}`.toLowerCase().includes(normalized),
        )
      : available;
    return [...filtered].sort((a, b) =>
      a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
    );
  }, [folders, currentFolderId, includeCurrent, query]);

  const trimmedQuery = query.trim();
  // Mirrors the note editor's project chip: offer create only when the query
  // would not duplicate an existing project name (case-insensitive).
  const hasExactMatch = folders.some(
    (folder) => folder.name.toLowerCase() === trimmedQuery.toLowerCase(),
  );
  const showCreate = Boolean(onCreateFolder) && trimmedQuery.length > 0 && !hasExactMatch;

  async function handleCommit() {
    if (sessions.length === 0 || !selectedId || submitting) return;
    setSubmitting(true);
    try {
      for (const session of sessions) {
        await onSetFolder(session.id, selectedId);
      }
    } catch {
      // The caller surfaced the error; keep the dialog open so a partial
      // move is visible and the user can retry.
      return;
    } finally {
      setSubmitting(false);
    }
    onMoved?.();
    onClose();
  }

  async function handleRemoveFromProject() {
    if (!onRemoveFolder || !currentFolderId || submitting) return;
    setSubmitting(true);
    try {
      for (const session of sessions) {
        await onRemoveFolder(session.id, currentFolderId);
      }
    } catch {
      // The caller surfaced the error; keep the dialog open for a retry.
      return;
    } finally {
      setSubmitting(false);
    }
    onMoved?.();
    onClose();
  }

  async function handleCreateAndAssign() {
    if (sessions.length === 0 || !onCreateFolder || !showCreate || submitting) return;
    setSubmitting(true);
    try {
      const folder = await onCreateFolder(trimmedQuery);
      // Creation failures surface through the caller's error handling; keep
      // the dialog open so the user can retry or pick an existing project.
      if (!folder) return;
      for (const session of sessions) {
        await onSetFolder(session.id, folder.id);
      }
    } catch {
      // Assignment failed after the project was created: the caller surfaced
      // the error, and the dialog stays open with the new project listed so
      // the user can retry instead of silently losing sessions.
      return;
    } finally {
      setSubmitting(false);
    }
    onMoved?.();
    onClose();
  }

  const title = isSingle
    ? hasCurrent
      ? t("notes.move.session.title")
      : t("notes.move.session.addTitle")
    : t("notes.move.session.bulkTitle", { count: sessions.length });
  const description = isSingle
    ? hasCurrent
      ? t("notes.move.session.currentDescription", { name: currentFolder?.name ?? "" })
      : t("notes.move.session.pickDescription")
    : t("notes.move.bulkDescription");
  const addMode = isSingle && !hasCurrent;
  const commitLabel = addMode ? t("common.add") : t("notes.list.move");
  const committingLabel = addMode ? t("notes.move.adding") : t("notes.move.moving");

  return (
    <Dialog
      open={open}
      onClose={() => {
        if (submitting) return;
        onClose();
      }}
      title={title}
      description={description}
      initialFocusSelector='input[name="move-session-search"]'
      footer={
        <>
          <button type="button" className="primary-action" onClick={onClose} disabled={submitting}>
            {t("common.cancel")}
          </button>
          <button
            type="button"
            className="primary-action primary-solid"
            onClick={() => void handleCommit()}
            disabled={submitting || !selectedId}
          >
            {submitting ? committingLabel : commitLabel}
          </button>
        </>
      }
    >
      <div className="move-note-dialog">
        <label className="add-notes-search">
          <IconMagnifyingGlass size={14} />
          <input
            ref={searchRef}
            type="search"
            name="move-session-search"
            placeholder={
              onCreateFolder ? t("notes.folderChip.searchOrCreate") : t("notes.move.searchProjects")
            }
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && showCreate) {
                event.preventDefault();
                void handleCreateAndAssign();
              }
            }}
            autoComplete="off"
          />
          {query ? (
            <button
              type="button"
              className="search-clear"
              aria-label={t("notes.folderChip.clearSearch")}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => {
                setQuery("");
                searchRef.current?.focus();
              }}
            >
              <IconCrossSmall size={13} />
            </button>
          ) : null}
        </label>
        {candidates.length > 0 ? (
          <ul className="add-notes-list" role="listbox">
            {candidates.map((folder) => {
              const isSelected = folder.id === selectedId;
              // The current project reads as checked; clicking it unfiles the
              // session - the same toggle as the note editor's project chip.
              const isCurrent = includeCurrent && folder.id === currentFolderId;
              return (
                <li key={folder.id}>
                  <button
                    type="button"
                    role="option"
                    aria-selected={isSelected || isCurrent}
                    aria-label={
                      isCurrent ? t("notes.move.removeFrom", { name: folder.name }) : undefined
                    }
                    className="add-notes-row"
                    data-selected={isSelected}
                    data-current={isCurrent || undefined}
                    disabled={submitting}
                    onClick={() =>
                      isCurrent ? void handleRemoveFromProject() : setSelectedId(folder.id)
                    }
                    onDoubleClick={isCurrent ? undefined : () => void handleCommit()}
                  >
                    <span className="add-notes-icon" aria-hidden>
                      <IconProjects size={14} />
                    </span>
                    <span className="add-notes-body">
                      <span className="add-notes-title">{folder.name}</span>
                      {folder.description ? (
                        <span className="add-notes-preview">{folder.description}</span>
                      ) : null}
                    </span>
                    <span className="add-notes-check" aria-hidden>
                      {isSelected || isCurrent ? <IconCheckmark2 size={12} /> : null}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        ) : showCreate ? null : (
          <p className="add-notes-empty">
            {folders.length === 0
              ? onCreateFolder
                ? t("notes.move.emptyCreate")
                : t("notes.move.emptyGoToProjects")
              : query.trim()
                ? t("notes.move.noMatch")
                : t("notes.move.noOther")}
          </p>
        )}
        {/* Create sits under the results: matches, if any, come first — the
            common case is filing into an existing project. */}
        {showCreate && candidates.length > 0 ? (
          <div className="add-notes-divider" aria-hidden />
        ) : null}
        {showCreate ? (
          <button
            type="button"
            className="add-notes-row add-notes-create"
            disabled={submitting}
            onClick={() => void handleCreateAndAssign()}
          >
            <span className="add-notes-icon" aria-hidden>
              <IconPlusMedium size={14} />
            </span>
            <span className="add-notes-body">
              <span className="add-notes-title">
                {t("notes.folderChip.create", { name: trimmedQuery })}
              </span>
            </span>
            <span className="add-notes-check" aria-hidden />
          </button>
        ) : null}
      </div>
    </Dialog>
  );
}
