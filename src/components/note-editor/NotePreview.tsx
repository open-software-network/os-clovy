import Placeholder from "@tiptap/extension-placeholder";
import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { IconBold } from "central-icons/IconBold";
import { IconBulletList } from "central-icons/IconBulletList";
import { IconH1 } from "central-icons/IconH1";
import { useEffect, useMemo, useRef, useState } from "react";
import { t as translate, useLocale, useT } from "../../i18n";

type NotePreviewProps = {
  noteId: string;
  markdown: string;
  // Tagged with the noteId the editor was created under so a late
  // blur (during note-switch teardown) can't silently overwrite a
  // different note's content.
  onChange: (noteId: string, markdown: string) => void;
  onBlur?: (noteId: string) => void;
  emptyPlaceholder?: string;
};

type NotePreviewSerializationState = {
  noteId: string;
  lastSerializedMarkdown: string;
  editorDirty: boolean;
  focusedExternalMarkdown: {
    editorMarkdown: string;
    externalMarkdown: string;
  } | null;
};

export function NotePreview({
  noteId,
  markdown,
  onChange,
  onBlur,
  emptyPlaceholder,
}: NotePreviewProps) {
  const t = useT();
  const locale = useLocale();
  // The editor is built once per note, so its placeholder and aria-label read
  // through a ref / the bare translator at render time rather than capturing
  // the strings of the render that created it.
  const emptyPlaceholderRef = useRef(emptyPlaceholder);
  emptyPlaceholderRef.current = emptyPlaceholder;
  const wrapRef = useRef<HTMLDivElement>(null);
  const [toolbar, setToolbar] = useState<{ x: number; y: number } | null>(null);
  const initialHtml = useMemo(() => markdownToHtml(markdown), [noteId]);
  const serializationStateRef = useRef<NotePreviewSerializationState>({
    noteId,
    lastSerializedMarkdown: markdown,
    editorDirty: false,
    focusedExternalMarkdown: null,
  });
  if (serializationStateRef.current.noteId !== noteId) {
    serializationStateRef.current = {
      noteId,
      lastSerializedMarkdown: markdown,
      editorDirty: false,
      focusedExternalMarkdown: null,
    };
  }
  // Capture the note-keyed object in each editor callback. A late blur from
  // the previous editor can then finish reconciling that note after a switch
  // without reading the new note's serialization state.
  const serializationState = serializationStateRef.current;

  const editor = useEditor(
    {
      extensions: [
        StarterKit.configure({
          heading: {
            levels: [1],
          },
        }),
        Placeholder.configure({
          placeholder: () =>
            emptyPlaceholderRef.current ?? translate("notes.editor.emptyPlaceholder"),
        }),
      ],
      content: initialHtml,
      editorProps: {
        attributes: () => ({
          class: "note-preview",
          role: "textbox",
          "aria-label": translate("notes.preview.label"),
          "aria-multiline": "true",
        }),
      },
      onUpdate: ({ editor, transaction }) => {
        if (!transaction.docChanged) return;
        const editorMarkdown = htmlToMarkdown(editor.view.dom);
        serializationState.lastSerializedMarkdown = editorMarkdown;
        serializationState.editorDirty = true;
        onChange(noteId, editorMarkdown);
      },
      onBlur: ({ editor }) => {
        // ProseMirror flushes an active IME composition through onUpdate
        // before invoking onBlur, so this cache includes its final text.
        // `noteId` here is the value at editor-creation time — the
        // useEditor dep list tears the editor down on note change, so
        // this closure always reflects the note the editor was bound
        // to, even when blur fires during teardown.
        const pendingExternalMarkdown = serializationState.focusedExternalMarkdown;
        serializationState.focusedExternalMarkdown = null;
        if (pendingExternalMarkdown) {
          if (serializationState.editorDirty) {
            const mergedMarkdown = mergeFocusedExternalMarkdown(
              serializationState.lastSerializedMarkdown,
              pendingExternalMarkdown,
            );
            onChange(noteId, mergedMarkdown);
          } else {
            // An external replacement is already persisted. A clean editor
            // must adopt it without writing its old focused snapshot back.
            editor.commands.setContent(markdownToHtml(pendingExternalMarkdown.externalMarkdown), {
              emitUpdate: false,
            });
            serializationState.lastSerializedMarkdown = pendingExternalMarkdown.externalMarkdown;
          }
        }
        serializationState.editorDirty = false;
        // onUpdate already queued the exact latest user edit. Blur only
        // reconciles a focused external append, then flushes that queue.
        onBlur?.(noteId);
      },
    },
    [noteId],
  );

  useEffect(() => {
    if (!editor) return;

    function updateToolbar() {
      setToolbar(getToolbarPosition(editor));
    }
    function hideToolbar() {
      setToolbar(null);
    }

    editor.on("selectionUpdate", updateToolbar);
    editor.on("focus", updateToolbar);
    editor.on("blur", hideToolbar);
    window.addEventListener("scroll", updateToolbar, true);

    return () => {
      editor.off("selectionUpdate", updateToolbar);
      editor.off("focus", updateToolbar);
      editor.off("blur", hideToolbar);
      window.removeEventListener("scroll", updateToolbar, true);
    };
  }, [editor]);

  // Re-render the editor's own DOM (aria-label, placeholder) when the
  // interface language changes: an empty transaction recomputes both without
  // touching the document.
  const renderedLocaleRef = useRef(locale);
  useEffect(() => {
    if (renderedLocaleRef.current === locale) return;
    renderedLocaleRef.current = locale;
    if (!editor || editor.isDestroyed) return;
    try {
      editor.view.dispatch(editor.state.tr.setMeta("addToHistory", false));
    } catch {
      // Mid-teardown; the next editor is built with the current language.
    }
  }, [editor, locale]);

  // Backend writes (transcribe → generate) arrive after the editor has
  // already mounted, so we have to pull new markdown in by hand. Skip
  // if the user is actively editing — clobbering focused content would
  // be a worse bug than the stale render. Guarded with try/catch and
  // an isDestroyed check so we never touch a torn-down view during
  // note-switch unmounts.
  useEffect(() => {
    if (!editor || editor.isDestroyed) return;
    try {
      const current = serializationState.lastSerializedMarkdown.trim();
      if (current === markdown.trim()) return;
      // Keep the focused document and selection stable, including when an
      // external append arrives just before the first keystroke. Clean blur
      // adopts the persisted text; dirty blur reconciles the local edit.
      if (editor.isFocused) {
        serializationState.focusedExternalMarkdown = {
          editorMarkdown: serializationState.lastSerializedMarkdown,
          externalMarkdown: markdown,
        };
        return;
      }
      editor.commands.setContent(markdownToHtml(markdown), {
        emitUpdate: false,
      });
      serializationState.lastSerializedMarkdown = markdown;
      serializationState.editorDirty = false;
      serializationState.focusedExternalMarkdown = null;
    } catch {
      // Editor is mid-teardown or view isn't ready yet — the next
      // mount will paint the correct content.
    }
  }, [editor, markdown, serializationState]);

  function applyFormat(command: "h1" | "bullet" | "bold") {
    if (!editor) return;
    if (command === "bold") {
      editor.chain().focus().toggleBold().run();
    } else if (command === "bullet") {
      editor.chain().focus().toggleBulletList().run();
    } else {
      editor.chain().focus().toggleHeading({ level: 1 }).run();
    }
    setToolbar(getToolbarPosition(editor));
  }

  return (
    <div ref={wrapRef} className="note-preview-wrap">
      <EditorContent editor={editor} />
      {toolbar ? (
        <div
          className="selection-toolbar"
          role="toolbar"
          aria-label={t("notes.preview.formatSelection")}
          style={{ left: toolbar.x, top: toolbar.y }}
          onMouseDown={(event) => event.preventDefault()}
        >
          <button
            type="button"
            data-active={editor?.isActive("heading", { level: 1 }) || undefined}
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => applyFormat("h1")}
            title={t("notes.preview.heading")}
            aria-label={t("notes.preview.heading")}
          >
            <IconH1 size={16} />
          </button>
          <button
            type="button"
            data-active={editor?.isActive("bulletList") || undefined}
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => applyFormat("bullet")}
            title={t("notes.preview.bulletList")}
            aria-label={t("notes.preview.bulletList")}
          >
            <IconBulletList size={16} />
          </button>
          <span className="divider" aria-hidden />
          <button
            type="button"
            data-active={editor?.isActive("bold") || undefined}
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => applyFormat("bold")}
            title={t("notes.preview.bold")}
            aria-label={t("notes.preview.bold")}
          >
            <IconBold size={16} />
          </button>
        </div>
      ) : null}
    </div>
  );
}

function mergeFocusedExternalMarkdown(
  editorMarkdown: string,
  pending: { editorMarkdown: string; externalMarkdown: string } | null,
) {
  if (!pending) return editorMarkdown;
  if (editorMarkdown.trim() === pending.externalMarkdown.trim()) {
    return editorMarkdown;
  }

  const base = pending.editorMarkdown.trim();
  const external = pending.externalMarkdown.trim();
  if (!external) return editorMarkdown;
  if (!base) {
    return editorMarkdown.trim()
      ? appendMarkdown(editorMarkdown, external)
      : pending.externalMarkdown;
  }
  // A replacement cannot be reduced to an appended suffix. Retain the local
  // edit and the external text rather than silently losing the user's work.
  if (!external.startsWith(base)) return appendMarkdown(editorMarkdown, external);

  const appended = external.slice(base.length).trim();
  if (!appended || editorMarkdown.trimEnd().endsWith(appended)) {
    return editorMarkdown;
  }
  return appendMarkdown(editorMarkdown, appended);
}

function appendMarkdown(existing: string, addition: string) {
  const existingTrimmed = existing.trimEnd();
  const additionTrimmed = addition.trim();
  if (!existingTrimmed) return additionTrimmed;
  if (!additionTrimmed) return existingTrimmed;
  return `${existingTrimmed}\n\n${additionTrimmed}`;
}

function getToolbarPosition(editor: Editor | null) {
  if (!editor || editor.state.selection.empty) return null;
  try {
    const { from, to } = editor.state.selection;
    const start = editor.view.coordsAtPos(from);
    const end = editor.view.coordsAtPos(to);
    return {
      x: (start.left + end.right) / 2,
      y: Math.min(start.top, end.top) - 8,
    };
  } catch {
    return null;
  }
}

/* ---- markdown <-> html (tiny subset: headings, lists, paragraphs, bold) -- */

function escapeHtml(value: string) {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function inlineToHtml(text: string) {
  return escapeHtml(text)
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*]+)\*/g, "$1<em>$2</em>");
}

function markdownToHtml(markdown: string): string {
  const lines = markdown.split("\n");
  const html: string[] = [];
  let listOpen = false;

  function closeList() {
    if (listOpen) {
      html.push("</ul>");
      listOpen = false;
    }
  }

  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (!line) {
      closeList();
      continue;
    }
    const heading = line.match(/^#{1,4}\s+(.+)$/);
    if (heading) {
      closeList();
      html.push(`<h1>${inlineToHtml(heading[1])}</h1>`);
      continue;
    }
    const item = line.match(/^[-*]\s+(.+)$/);
    if (item) {
      if (!listOpen) {
        html.push("<ul>");
        listOpen = true;
      }
      html.push(`<li>${inlineToHtml(item[1])}</li>`);
      continue;
    }
    closeList();
    html.push(`<p>${inlineToHtml(line)}</p>`);
  }
  closeList();
  return html.join("");
}

function inlineToMarkdown(node: Node): string {
  if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
  if (node.nodeType !== Node.ELEMENT_NODE) return "";
  const el = node as HTMLElement;
  const inner = Array.from(el.childNodes).map(inlineToMarkdown).join("");
  const tag = el.tagName.toLowerCase();
  if (tag === "strong" || tag === "b") return `**${inner}**`;
  if (tag === "em" || tag === "i") return `*${inner}*`;
  if (tag === "br") return "\n";
  return inner;
}

export function htmlToMarkdown(root: HTMLElement): string {
  const blocks: string[] = [];
  for (const node of Array.from(root.childNodes)) {
    if (node.nodeType === Node.TEXT_NODE) {
      const text = node.textContent?.trim();
      if (text) blocks.push(text);
      continue;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) continue;
    const el = node as HTMLElement;
    const tag = el.tagName.toLowerCase();
    if (tag === "h1") {
      blocks.push(`# ${inlineToMarkdown(el).trim()}`);
    } else if (tag === "ul" || tag === "ol") {
      for (const li of Array.from(el.querySelectorAll("li"))) {
        const text = inlineToMarkdown(li).trim();
        if (text) blocks.push(`- ${text}`);
      }
    } else {
      const text = inlineToMarkdown(el).trim();
      if (text) blocks.push(text);
    }
  }
  return blocks.join("\n\n");
}
