import { IconCheckmark2 } from "central-icons-filled/IconCheckmark2";
import { IconBubble3 } from "central-icons/IconBubble3";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { useEffect, useMemo, useRef, useState } from "react";
import type { FolderDto } from "../../lib/tauri";
import type { AgentSessionDto } from "../../lib/agent-runtime-contract";
import { useT } from "../../i18n";
import { Dialog } from "../ui/Dialog";

type AddSessionsToProjectDialogProps = {
  open: boolean;
  onClose: () => void;
  folder: FolderDto;
  sessions: AgentSessionDto[];
  /** sessionId -> project ids, used to hide sessions already in the project. */
  sessionFolderIds: Record<string, string[]>;
  /** Called once per session when the user commits the selection. */
  onAdd: (sessionId: string) => Promise<unknown> | void;
};

export function AddSessionsToProjectDialog({
  open,
  onClose,
  folder,
  sessions,
  sessionFolderIds,
  onAdd,
}: AddSessionsToProjectDialogProps) {
  const t = useT();
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [submitting, setSubmitting] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSelected(new Set());
    setSubmitting(false);
  }, [open]);

  const candidates = useMemo(() => {
    const available = sessions.filter(
      (session) => !(sessionFolderIds[session.id] ?? []).includes(folder.id),
    );
    const normalized = query.trim().toLowerCase();
    if (!normalized) return available;
    return available.filter((session) => session.title.toLowerCase().includes(normalized));
  }, [sessions, sessionFolderIds, folder.id, query]);

  function toggle(sessionId: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(sessionId)) next.delete(sessionId);
      else next.add(sessionId);
      return next;
    });
  }

  async function handleSubmit() {
    if (selected.size === 0 || submitting) return;
    setSubmitting(true);
    try {
      // Commit serially so backend assigns are deterministic.
      for (const sessionId of selected) {
        await onAdd(sessionId);
      }
      onClose();
    } finally {
      setSubmitting(false);
    }
  }

  const count = selected.size;

  return (
    <Dialog
      open={open}
      onClose={() => {
        if (submitting) return;
        onClose();
      }}
      title={t("notes.add.sessions.title", { name: folder.name })}
      description={t("notes.add.sessions.description")}
      initialFocusSelector='input[name="add-sessions-search"]'
      footer={
        <>
          <button type="button" className="primary-action" onClick={onClose} disabled={submitting}>
            {t("common.cancel")}
          </button>
          <button
            type="button"
            className="primary-action primary-solid"
            onClick={() => void handleSubmit()}
            disabled={submitting || count === 0}
          >
            {submitting ? t("notes.move.adding") : t("notes.add.sessions.commit", { count })}
          </button>
        </>
      }
    >
      <div className="add-notes-dialog">
        <label className="add-notes-search">
          <IconMagnifyingGlass size={14} />
          <input
            ref={searchRef}
            type="search"
            name="add-sessions-search"
            placeholder={t("notes.add.sessions.search")}
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
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
          <ul className="add-notes-list" role="listbox" aria-multiselectable>
            {candidates.map((session) => {
              const isSelected = selected.has(session.id);
              return (
                <li key={session.id}>
                  <button
                    type="button"
                    role="option"
                    aria-selected={isSelected}
                    className="add-notes-row"
                    data-selected={isSelected}
                    onClick={() => toggle(session.id)}
                  >
                    <span className="add-notes-icon" aria-hidden>
                      <IconBubble3 size={14} />
                    </span>
                    <span className="add-notes-body">
                      <span className="add-notes-title">
                        {session.title.trim() || t("chat.session.untitled")}
                      </span>
                      <span className="add-notes-preview">
                        {session.source === "legacy_routine"
                          ? t("notes.add.sessions.importedRoutine")
                          : t("notes.add.sessions.conversation")}
                      </span>
                    </span>
                    <span className="add-notes-check" aria-hidden>
                      {isSelected ? <IconCheckmark2 size={12} /> : null}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        ) : (
          <p className="add-notes-empty">
            {sessions.some((session) => !(sessionFolderIds[session.id] ?? []).includes(folder.id))
              ? t("notes.add.sessions.noMatch")
              : t("notes.add.sessions.allIn")}
          </p>
        )}
      </div>
    </Dialog>
  );
}
