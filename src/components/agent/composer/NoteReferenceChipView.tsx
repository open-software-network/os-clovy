import { NodeViewWrapper, type NodeViewProps } from "@tiptap/react";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconNoteText } from "central-icons/IconNoteText";

import { useT } from "../../../i18n";
import { displayNoteTitle } from "./noteReference";

export function NoteReferenceChipView({ node, deleteNode }: NodeViewProps) {
  const t = useT();
  const title = typeof node.attrs.title === "string" ? node.attrs.title : "";
  const noteId = typeof node.attrs.noteId === "string" ? node.attrs.noteId : "";

  return (
    <NodeViewWrapper
      as="span"
      className="agent-note-reference-chip"
      data-note-id={noteId}
      contentEditable={false}
    >
      <span className="agent-note-reference-chip-icon" aria-hidden="true">
        <IconNoteText size={10} />
      </span>
      <span className="agent-note-reference-chip-title">{displayNoteTitle(title)}</span>
      <button
        type="button"
        className="agent-note-reference-chip-remove"
        aria-label={t("chat.composer.removeNoteReference")}
        // preventDefault so the mousedown doesn't move the editor selection out
        // of the chip before deleteNode runs.
        onMouseDown={(event) => {
          event.preventDefault();
          deleteNode();
        }}
      >
        <IconCrossSmall size={10} aria-hidden />
      </button>
    </NodeViewWrapper>
  );
}
