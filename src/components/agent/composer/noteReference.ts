import Mention from "@tiptap/extension-mention";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import { PluginKey, TextSelection } from "@tiptap/pm/state";
import type { EditorState, Transaction } from "@tiptap/pm/state";
import type { Editor } from "@tiptap/react";
import { ReactNodeViewRenderer, ReactRenderer } from "@tiptap/react";

import { NoteReferenceChipView } from "./NoteReferenceChipView";
import {
  NoteSuggestionList,
  type NoteSuggestionListHandle,
  type NoteSuggestionListProps,
} from "./NoteSuggestionList";
import { t } from "../../../i18n";
import { listNotes, type NoteListItemDto } from "../../../lib/tauri";

/** Node name for the inline note reference chip. It stays distinct from the
 * category chip because note references serialize into prompt text and may
 * appear many times in one message. */
export const NOTE_REFERENCE_NODE = "noteReference";

const TRIGGER_CHAR = "@";
const NOTE_SUGGESTION_LIMIT = 8;
const NOTE_REFERENCE_SUGGESTION_PLUGIN_KEY = new PluginKey("agentNoteReferenceSuggestion");

export type NoteReferenceInput = {
  id: string;
  title: string;
};

export type NoteReferenceOptions = {
  fetchNotes?: () => Promise<NoteListItemDto[]>;
};

/** Fallback matches the note title input's placeholder. */
export function displayNoteTitle(title: string): string {
  return title.trim() || t("chat.composer.newNote");
}

export function noteReferenceToken(ref: NoteReferenceInput): string {
  const title = ref.title.replace(/\s+/g, " ").replaceAll('"', "").trim().slice(0, 80);
  return title ? `@note:${ref.id} ("${title}")` : `@note:${ref.id}`;
}

export function filterNoteSuggestions(
  notes: NoteListItemDto[],
  query: string,
  limit: number,
): NoteListItemDto[] {
  const cappedLimit = Math.max(0, limit);
  if (cappedLimit === 0) return [];
  const needle = query.trim().toLowerCase();
  if (!needle) return notes.slice(0, cappedLimit);
  // Match title OR preview, not title alone: most notes are untitled ("New
  // note") with nothing to match, so a title-only search reads as broken.
  // Title hits rank above preview-only hits; recency order holds within each
  // tier (the input list is already most-recent-first).
  const titleMatches: NoteListItemDto[] = [];
  const previewMatches: NoteListItemDto[] = [];
  for (const note of notes) {
    if (note.title.toLowerCase().includes(needle)) titleMatches.push(note);
    else if (note.preview.toLowerCase().includes(needle)) previewMatches.push(note);
  }
  return [...titleMatches, ...previewMatches].slice(0, cappedLimit);
}

/** A tiptap command that inserts a note reference atom at `range` when the
 * suggestion palette owns an "@query" span, or at the current selection when
 * called imperatively. Unlike category tags, note chips are references inside
 * the prompt, so multiple chips are allowed and no existing chip is cleared. */
function insertNoteReferenceCommand(ref: NoteReferenceInput, range?: { from: number; to: number }) {
  return ({
    tr,
    state,
    dispatch,
  }: {
    tr: Transaction;
    state: EditorState;
    dispatch?: (tr: Transaction) => void;
  }) => {
    const chip = state.schema.nodes[NOTE_REFERENCE_NODE]?.create({
      noteId: ref.id,
      title: ref.title,
    });
    if (!chip) return false;
    if (!dispatch) return true;

    const from = tr.mapping.map(range ? range.from : state.selection.from);
    const to = tr.mapping.map(range ? range.to : state.selection.to);
    tr.replaceWith(from, to, chip);

    // Land the caret on text after the chip, adding a space when the chip
    // would otherwise butt against the next character (or the doc end).
    const afterChip = from + chip.nodeSize;
    const $after = tr.doc.resolve(afterChip);
    const next = $after.nodeAfter;
    if (!next?.isText || !next.text?.startsWith(" ")) {
      tr.insert(afterChip, state.schema.text(" "));
    }
    tr.setSelection(TextSelection.create(tr.doc, afterChip + 1));
    dispatch(tr.scrollIntoView());
    return true;
  };
}

/** Inserts a note reference chip at the current selection. Used by follow-up
 * composer affordances that already know which note the user picked. */
export function insertNoteReference(editor: Editor, ref: NoteReferenceInput) {
  editor.chain().focus().command(insertNoteReferenceCommand(ref)).run();
}

const NoteReferenceBase = Mention.extend({
  name: NOTE_REFERENCE_NODE,

  addAttributes() {
    return {
      noteId: {
        default: null,
        parseHTML: (element) => element.getAttribute("data-note-id"),
        renderHTML: (attributes) =>
          attributes.noteId ? { "data-note-id": attributes.noteId as string } : {},
      },
      title: {
        default: "",
        parseHTML: (element) => element.getAttribute("data-title") ?? "",
        renderHTML: (attributes) => ({ "data-title": (attributes.title as string) ?? "" }),
      },
    };
  },

  renderText({ node }: { node: ProseMirrorNode }) {
    return noteReferenceToken({
      id: typeof node.attrs.noteId === "string" ? node.attrs.noteId : "",
      title: typeof node.attrs.title === "string" ? node.attrs.title : "",
    });
  },

  addNodeView() {
    return ReactNodeViewRenderer(NoteReferenceChipView);
  },
});

// The list_notes command defaults to the 100 most recent notes, which reads
// as "that note doesn't exist" in the palette for heavy users. 500 keeps the
// fetch cheap (one palette-open, lightweight rows) while covering realistic
// libraries; server-side search is the follow-up if that ever falls short.
const PALETTE_FETCH_LIMIT = 500;

async function defaultFetchNotes() {
  const response = await listNotes(undefined, PALETTE_FETCH_LIMIT);
  return response.items;
}

export function createNoteReference(options: NoteReferenceOptions = {}) {
  const fetchNotes = options.fetchNotes ?? defaultFetchNotes;
  let paletteNotesPromise: Promise<NoteListItemDto[]> | null = null;

  function notesForPaletteSession() {
    paletteNotesPromise ??= fetchNotes();
    return paletteNotesPromise;
  }

  function clearPaletteSession() {
    paletteNotesPromise = null;
  }

  return NoteReferenceBase.configure({
    deleteTriggerWithBackspace: true,
    renderHTML({ node }) {
      return [
        "span",
        {
          class: "agent-note-reference-chip",
          "data-note-id": (node.attrs.noteId as string) ?? "",
          "data-title": (node.attrs.title as string) ?? "",
        },
        (node.attrs.title as string) ?? "",
      ];
    },
    suggestion: {
      char: TRIGGER_CHAR,
      pluginKey: NOTE_REFERENCE_SUGGESTION_PLUGIN_KEY,
      // Keep the v1 matcher narrow: email-like mid-word "@" text should not
      // leave a sticky note palette behind.
      allowSpaces: false,
      items: async ({ query }) =>
        filterNoteSuggestions(await notesForPaletteSession(), query, NOTE_SUGGESTION_LIMIT),
      command: ({ editor, range, props }) => {
        const item = props as NoteListItemDto;
        editor
          .chain()
          .focus()
          .command(insertNoteReferenceCommand({ id: item.id, title: item.title }, range))
          .run();
      },
      render: () => {
        let renderer: ReactRenderer<NoteSuggestionListHandle, NoteSuggestionListProps> | null =
          null;
        let host: HTMLDivElement | null = null;
        let latestProps: {
          command: NoteSuggestionListProps["command"];
          editor: Editor;
          clientRect?: (() => DOMRect | null) | null;
        } | null = null;
        let ownerDocument: Document | null = null;
        let ownerWindow: Window | null = null;

        function position(props: { clientRect?: (() => DOMRect | null) | null; editor: Editor }) {
          if (!host || !props.clientRect) return;
          const rect = props.clientRect();
          if (!rect) return;
          const viewport = props.editor.view.dom.ownerDocument.defaultView ?? window;
          const gap = 6;
          const pad = 8;
          const composerBox = props.editor.view.dom.closest<HTMLElement>(".agent-composer-box");
          const composerRect = composerBox?.getBoundingClientRect();
          const width = Math.min(
            composerRect?.width ?? host.getBoundingClientRect().width,
            viewport.innerWidth - pad * 2,
          );
          host.style.setProperty("--agent-category-menu-width", `${width}px`);
          const maxLeft = viewport.innerWidth - width - pad;
          const left = Math.min(
            Math.max(composerRect?.left ?? rect.left, pad),
            Math.max(pad, maxLeft),
          );
          const anchorRect = composerRect ?? rect;
          const belowTop = anchorRect.bottom + gap;
          const belowSpace = viewport.innerHeight - belowTop - pad;
          const aboveSpace = anchorRect.top - gap - pad;
          const hostRect = host.getBoundingClientRect();
          const hasMeasuredHeight = hostRect.height > 0;
          const fitsBelow = hasMeasuredHeight && belowSpace >= hostRect.height;
          const fitsAbove = hasMeasuredHeight && aboveSpace >= hostRect.height;
          const placeBelow = fitsBelow || (!fitsAbove && belowSpace >= aboveSpace);
          const maxHeight = Math.max(0, Math.min(placeBelow ? belowSpace : aboveSpace, 280));

          host.style.setProperty("--agent-category-menu-max-height", `${maxHeight}px`);
          if (placeBelow) {
            host.style.bottom = "";
            host.style.top = `${Math.max(belowTop, pad)}px`;
          } else {
            host.style.top = "";
            host.style.bottom = `${Math.max(viewport.innerHeight - anchorRect.top + gap, pad)}px`;
          }
          host.style.left = `${left}px`;
        }

        function positionLatest() {
          if (latestProps) position(latestProps);
        }

        function updateLatestProps(props: {
          command: unknown;
          editor: Editor;
          clientRect?: (() => DOMRect | null) | null;
        }) {
          latestProps = {
            ...props,
            command: props.command as NoteSuggestionListProps["command"],
          };
        }

        function dismissFromPointerDown(event: PointerEvent) {
          const target = event.target;
          if (!(target instanceof Node) || host?.contains(target)) return;
          const view = latestProps?.editor.view;
          if (!view) return;
          view.dispatch(
            view.state.tr.setMeta(NOTE_REFERENCE_SUGGESTION_PLUGIN_KEY, {
              exit: true,
            }),
          );
        }

        function cleanupPopover() {
          renderer?.destroy();
          ownerDocument?.removeEventListener("pointerdown", dismissFromPointerDown, true);
          ownerWindow?.removeEventListener("resize", positionLatest);
          ownerWindow?.visualViewport?.removeEventListener("resize", positionLatest);
          host?.remove();
          renderer = null;
          host = null;
          latestProps = null;
          ownerDocument = null;
          ownerWindow = null;
          clearPaletteSession();
        }

        return {
          onStart(props) {
            updateLatestProps(props);
            renderer = new ReactRenderer(NoteSuggestionList, {
              props: { items: props.items, command: props.command },
              editor: props.editor,
            });
            host = document.createElement("div");
            // Reuse the category-menu host contract so Enter-to-submit lets an
            // open note palette consume Enter, just like the "/" palette.
            host.className = "agent-category-menu-host agent-note-reference-menu-host";
            host.appendChild(renderer.element);
            document.body.appendChild(host);
            ownerDocument = props.editor.view.dom.ownerDocument;
            ownerWindow = ownerDocument.defaultView;
            ownerDocument.addEventListener("pointerdown", dismissFromPointerDown, true);
            ownerWindow?.addEventListener("resize", positionLatest);
            ownerWindow?.visualViewport?.addEventListener("resize", positionLatest);
            position(props);
          },
          onUpdate(props) {
            updateLatestProps(props);
            renderer?.updateProps({
              items: props.items,
              command: props.command,
            });
            position(props);
          },
          onKeyDown(props) {
            if (props.event.key === "Escape") {
              cleanupPopover();
              return true;
            }
            return renderer?.ref?.onKeyDown(props.event) ?? false;
          },
          onExit() {
            cleanupPopover();
          },
        };
      },
    },
  });
}
