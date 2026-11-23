import { IconAudio } from "central-icons/IconAudio";
import { IconDotGrid1x3Horizontal } from "central-icons/IconDotGrid1x3Horizontal";
import { IconFilePdf } from "central-icons/IconFilePdf";
import { IconReference } from "central-icons/IconReference";
import { IconTrashCan } from "central-icons/IconTrashCan";
import { useEffect, useRef, useState } from "react";

import { useT } from "../../i18n";
import { noteReferenceToken } from "../agent/composer/noteReference";
import { toast } from "../ui/Toaster";

export function NoteHeaderActions({
  noteId,
  noteTitle,
  onExportPdf,
  onDownloadAudio,
  onDelete,
}: {
  noteId: string;
  noteTitle: string;
  /** Opens the system print sheet with a PDF-ready version of the note. */
  onExportPdf?: () => void;
  /** Downloads the note's finalized audio artifacts. */
  onDownloadAudio?: () => void;
  /** Opens the delete-note confirmation. */
  onDelete?: () => void;
}) {
  return (
    <div className="note-header-actions">
      <NoteOverflowMenu
        noteId={noteId}
        noteTitle={noteTitle}
        onExportPdf={onExportPdf}
        onDownloadAudio={onDownloadAudio}
        onDelete={onDelete}
      />
    </div>
  );
}

function NoteOverflowMenu({
  noteId,
  noteTitle,
  onExportPdf,
  onDownloadAudio,
  onDelete,
}: {
  noteId: string;
  noteTitle: string;
  onExportPdf?: () => void;
  onDownloadAudio?: () => void;
  onDelete?: () => void;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function handlePointerDown(event: MouseEvent) {
      if (!wrapRef.current?.contains(event.target as Node)) setOpen(false);
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [open]);

  async function handleCopyReference() {
    try {
      await navigator.clipboard.writeText(noteReferenceToken({ id: noteId, title: noteTitle }));
      toast(t("notes.header.referenceCopied"));
    } catch {
      // Clipboard API can fail in restricted contexts; stay silent so retrying
      // the same menu action remains the least disruptive recovery.
    }
  }

  return (
    <div className="note-actions-menu-wrap" ref={wrapRef}>
      <button
        type="button"
        className="icon-button note-actions-menu-trigger"
        aria-label={t("notes.header.actions")}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <IconDotGrid1x3Horizontal size={16} />
      </button>
      {open ? (
        <div className="sidebar-identity-menu note-actions-menu" role="menu">
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              void handleCopyReference();
            }}
          >
            <IconReference size={14} />
            {t("notes.header.copyReference")}
          </button>
          {onExportPdf ? (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setOpen(false);
                onExportPdf();
              }}
            >
              <IconFilePdf size={14} />
              {t("notes.header.exportPdf")}
            </button>
          ) : null}
          {onDownloadAudio ? (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setOpen(false);
                onDownloadAudio();
              }}
            >
              <IconAudio size={14} />
              {t("notes.header.downloadAudio")}
            </button>
          ) : null}
          {onDelete ? (
            <button
              type="button"
              role="menuitem"
              className="destructive"
              onClick={() => {
                setOpen(false);
                onDelete();
              }}
            >
              <IconTrashCan size={14} />
              {t("notes.header.deleteNote")}
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
