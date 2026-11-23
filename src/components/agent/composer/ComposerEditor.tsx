import Placeholder from "@tiptap/extension-placeholder";
import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { Fragment, Slice, type Node as ProseMirrorNode } from "@tiptap/pm/model";
import { closeHistory } from "@tiptap/pm/history";
import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";

import {
  CATEGORY_CHIP_NODE,
  CATEGORY_SKILLS_CHANGED_EVENT,
  categoryFromDoc,
  createCategoryChip,
  insertReportCategory,
} from "./categoryChip";
import {
  NOTE_REFERENCE_NODE,
  createNoteReference,
  insertNoteReference,
  noteReferenceToken,
  type NoteReferenceInput,
} from "./noteReference";
import type { ReportCategory } from "./reportCategory";
import { useT } from "../../../i18n";
import type { AgentSkillInfo } from "../../../lib/tauri";
import type { BuiltinComposerSlashCommandName } from "../../../lib/agent-composer-slash-commands";

type SetContentOptions = {
  selectPlaceholder?: boolean;
  focus?: boolean;
  changeKey?: string | null;
};

export type ComposerEditorHandle = {
  focus: () => void;
  /** Publishes the current serialized document immediately. Returns false
   * while an IME composition is active, when reading the document would
   * capture an intermediate composition state. */
  flushPendingChange: (options?: {
    changeKey?: string | null;
    persistWithoutRender?: boolean;
  }) => boolean;
  clear: () => void;
  /** Replaces the whole document with plain text plus an optional leading
   * category chip. Used to prefill (hero shortcuts, HUD replies) and to restore
   * the composer after a failed send. With `selectPlaceholder`, the first
   * `<placeholder>` token's brackets are stripped and the bare phrase left
   * selected so the user can overtype it in place. */
  setContent: (text: string, category?: ReportCategory | null, options?: SetContentOptions) => void;
  /** Inserts or swaps the legacy message category tag at the caret. */
  insertCategory: (category: ReportCategory) => void;
  /** Inserts a note reference chip at the caret. Multiple references can
   * coexist because they serialize into the prompt text. */
  insertNoteReference: (ref: NoteReferenceInput) => void;
  /** Replaces the current selection with literal text without changing focus. */
  insertPlainText: (text: string) => boolean;
  isFocused: () => boolean;
  isEmpty: () => boolean;
};

type ComposerEditorProps = {
  placeholder: string;
  skills?: AgentSkillInfo[] | null;
  changeKey?: string | null;
  onChange: (
    text: string,
    category: ReportCategory | null,
    changeKey: string | null | undefined,
  ) => void;
  onPendingChangePersist?: (
    text: string,
    category: ReportCategory | null,
    changeKey: string | null | undefined,
  ) => void;
  onSubmit: () => void;
  onFocusChange?: (focused: boolean) => void;
  /** Reports cheap document empty/non-empty transitions without serializing
   * the prompt, so Send can react immediately during the trailing window. */
  onContentChange?: (hasContent: boolean) => void;
  /** Returns true when the host handles the selected command as an immediate
   * action instead of inserting its slash text into the draft. */
  onBuiltinSlashCommand?: (name: BuiltinComposerSlashCommandName) => boolean;
  /** Builtin commands the host surface does not offer in the slash menu. */
  hiddenBuiltinSlashCommands?: readonly BuiltinComposerSlashCommandName[];
  /** Hands the live editor up to the parent (e.g. so the composer box can read
   * its DOM element for layout). */
  onReady?: (editor: Editor) => void;
  /** Test seam for counting full-document serialization without changing the
   * production serializer. */
  testOnlySerializePlainText?: (doc: ProseMirrorNode) => string;
  /** Test seam for keeping the trailing timer pending during menu interaction. */
  testOnlyChangeDelayMs?: number;
};

export const COMPOSER_CHANGE_DELAY_MS = 75;

export function composerScrollMargin(fadeSize: string, paddingBottom: string): number {
  const parsedFadeSize = Number.parseFloat(fadeSize);
  const parsedPadding = Number.parseFloat(paddingBottom);
  return Math.max(
    Number.isFinite(parsedFadeSize) ? parsedFadeSize : 0,
    Number.isFinite(parsedPadding) ? parsedPadding : 6,
  );
}

export type ComposerDocumentEdge = "start" | "end";

export function composerDocumentEdge(
  selectionHead: number,
  documentEnd: number,
): ComposerDocumentEdge | null {
  if (selectionHead <= 1) return "start";
  if (selectionHead >= documentEnd - 1) return "end";
  return null;
}

type PendingEditorAction =
  | { type: "focus" }
  | { type: "clear" }
  | {
      type: "setContent";
      text: string;
      category?: ReportCategory | null;
      options?: SetContentOptions;
    }
  | { type: "insertCategory"; category: ReportCategory }
  | { type: "insertNoteReference"; noteReference: NoteReferenceInput };

/** Serializes the doc to the plain string sent to Clovy: paragraph and
 * hard-break boundaries become newlines, the category chip contributes
 * nothing, and note reference atoms emit the stable token Clovy resolves via
 * the note context tool. */
export function serializePlainText(doc: ProseMirrorNode): string {
  return doc.textBetween(0, doc.content.size, "\n", (leaf) => {
    if (leaf.type.name === "hardBreak") return "\n";
    if (leaf.type.name === NOTE_REFERENCE_NODE) {
      return noteReferenceToken({
        id: typeof leaf.attrs.noteId === "string" ? leaf.attrs.noteId : "",
        title: typeof leaf.attrs.title === "string" ? leaf.attrs.title : "",
      });
    }
    return "";
  });
}

/** Tiptap reports an editor as destroyed both before its view mounts and after
 * that view unmounts. Treat both lifecycle windows as unavailable so callers
 * never reach the throwing `editor.view` proxy. */
function editorHasView(editor: Editor | null): editor is Editor {
  return editor !== null && !editor.isDestroyed;
}

/** Focuses the editor with the caret at the end, synchronously. tiptap's
 * `focus` command defers the actual `view.focus()` to a requestAnimationFrame
 * (for scroll handling); that deferral can land after the user has moved on
 * and steal focus back from, say, a menu they just opened. Focusing the view
 * directly keeps it immediate. */
function focusEnd(editor: Editor | null) {
  if (!editorHasView(editor)) return;
  editor.commands.setTextSelection(editor.state.doc.content.size);
  editor.view.focus();
}

/** Stops at the first sendable leaf without allocating the serialized prompt.
 * This is normally constant-time for a non-empty draft, while preserving the
 * existing rule that whitespace and a report-category chip alone cannot send. */
function hasSubmittableContent(doc: ProseMirrorNode): boolean {
  let hasContent = false;
  doc.descendants((node) => {
    if (node.type.name === NOTE_REFERENCE_NODE || (node.isText && /\S/u.test(node.text ?? ""))) {
      hasContent = true;
      return false;
    }
    return !hasContent;
  });
  return hasContent;
}

/** Prepares a prefilled single-paragraph prompt for staging: strips the angle
 * brackets off the first `<placeholder>` token (authoring syntax that should
 * never reach the user's eyes) and maps the bare phrase to its ProseMirror
 * selection range so a hero shortcut can highlight it for overtyping. The
 * paragraph's opening boundary occupies position 0, so a string index `i`
 * maps to document position `i + 1`. Null when there is no placeholder. */
export function stripPlaceholder(text: string): { text: string; from: number; to: number } | null {
  const start = text.indexOf("<");
  const end = text.indexOf(">");
  if (start < 0 || end <= start) return null;
  return {
    text: text.slice(0, start) + text.slice(start + 1, end) + text.slice(end + 1),
    from: start + 1,
    to: end,
  };
}

/** Splits a line into text nodes and note-reference chips. Drafts persist as
 * the serialized plain string, so a restored draft carries reference tokens
 * as text; rebuilding the chip here keeps the pill UX across restores. The
 * round-trip is lossless because serializePlainText re-emits the token. */
function inlineContent(line: string) {
  const nodes: Array<Record<string, unknown>> = [];
  const tokenPattern = /@note:([\w-]+)(?: \("([^"]*)"\))?/g;
  let consumed = 0;
  for (const match of line.matchAll(tokenPattern)) {
    if (match.index > consumed) {
      nodes.push({ type: "text", text: line.slice(consumed, match.index) });
    }
    nodes.push({
      type: NOTE_REFERENCE_NODE,
      attrs: { noteId: match[1], title: match[2] ?? "" },
    });
    consumed = match.index + match[0].length;
  }
  if (consumed < line.length) {
    nodes.push({ type: "text", text: line.slice(consumed) });
  }
  return nodes;
}

export function buildDoc(
  text: string,
  category?: ReportCategory | null,
  options?: { rehydrateNoteTokens?: boolean },
) {
  // Placeholder-staged prefills skip rehydration: stripPlaceholder maps raw
  // string indices to doc positions, which only holds while every character
  // stays a size-1 text position — a chip atom would shift the selection.
  const rehydrate = options?.rehydrateNoteTokens !== false;
  const paragraphs = text.split("\n").map((line) => ({
    type: "paragraph",
    // A text node may not be empty, so a blank line is an empty paragraph.
    content: rehydrate ? inlineContent(line) : line ? [{ type: "text", text: line }] : [],
  }));
  if (paragraphs.length === 0) paragraphs.push({ type: "paragraph", content: [] });
  if (category) {
    const first = paragraphs[0];
    first.content = [
      { type: CATEGORY_CHIP_NODE, attrs: { category } } as never,
      { type: "text", text: " " },
      ...first.content,
    ];
  }
  return { type: "doc", content: paragraphs };
}

function setEditorContent(
  editor: Editor,
  text: string,
  category?: ReportCategory | null,
  options?: SetContentOptions,
) {
  const staged = options?.selectPlaceholder && !category ? stripPlaceholder(text) : null;
  editor.commands.setContent(
    buildDoc(staged?.text ?? text, category, { rehydrateNoteTokens: !staged }),
    {
      emitUpdate: true,
    },
  );
  // A hero shortcut authors a "<placeholder>" token; the brackets are
  // stripped before the text hits the document and the bare phrase left
  // selected (rather than parking the caret at the end) so typing
  // overtypes it in place.
  const range = staged ? { from: staged.from, to: staged.to } : null;
  const shouldFocus = options?.focus !== false;
  if (range) {
    editor.commands.setTextSelection(range);
    if (shouldFocus) editor.view.focus();
  } else if (shouldFocus) {
    focusEnd(editor);
  }
}

function applyEditorAction(
  editor: Editor,
  action: PendingEditorAction,
  pendingChangeKeyRef: { current: string | null | undefined },
) {
  switch (action.type) {
    case "focus":
      focusEnd(editor);
      break;
    case "clear":
      editor.commands.clearContent(true);
      break;
    case "setContent":
      setEditorContent(editor, action.text, action.category, action.options);
      // setContent emits its update synchronously, but a session-switch render
      // may not have updated the prop-backed key yet. Keep queued and immediate
      // replacements attributed to the destination draft explicitly.
      if (action.options && "changeKey" in action.options) {
        pendingChangeKeyRef.current = action.options.changeKey;
      }
      break;
    case "insertCategory":
      insertReportCategory(editor, action.category);
      break;
    case "insertNoteReference":
      insertNoteReference(editor, action.noteReference);
      break;
  }
}

function queueEditorAction(
  pendingActions: PendingEditorAction[],
  action: PendingEditorAction,
): PendingEditorAction[] {
  if (action.type === "setContent" || action.type === "clear") {
    // A replacement supersedes earlier document mutations. Preserve a
    // standalone focus request, but make content last-write-wins.
    return [...pendingActions.filter(({ type }) => type === "focus"), action];
  }
  if (action.type === "focus" && pendingActions.some(({ type }) => type === "focus")) {
    return pendingActions;
  }
  return [...pendingActions, action];
}

function flushEditorActions(
  editor: Editor | null,
  pendingActionsRef: { current: PendingEditorAction[] },
  pendingChangeKeyRef: { current: string | null | undefined },
) {
  if (!editorHasView(editor) || pendingActionsRef.current.length === 0) return;
  const pendingActions = pendingActionsRef.current;
  pendingActionsRef.current = [];
  for (const action of pendingActions) {
    applyEditorAction(editor, action, pendingChangeKeyRef);
  }
}

export const ComposerEditor = forwardRef<ComposerEditorHandle, ComposerEditorProps>(
  (
    {
      placeholder,
      skills,
      changeKey,
      onChange,
      onPendingChangePersist,
      onSubmit,
      onFocusChange,
      onContentChange,
      onBuiltinSlashCommand,
      hiddenBuiltinSlashCommands,
      onReady,
      testOnlySerializePlainText,
      testOnlyChangeDelayMs,
    },
    ref,
  ) => {
    const t = useT();
    const editorLabel = t("chat.composer.messageClovy");
    // The editor is created once, so its attributes read the label through a
    // ref (ProseMirror re-evaluates them on every view update) and a language
    // change patches the live DOM below.
    const editorLabelRef = useRef(editorLabel);
    editorLabelRef.current = editorLabel;
    const frameRef = useRef<HTMLDivElement | null>(null);
    const pendingEditorActionsRef = useRef<PendingEditorAction[]>([]);
    const skillsRef = useRef(skills);
    const pendingChangeTimerRef = useRef<number | null>(null);
    const pendingChangeEditorRef = useRef<Editor | null>(null);
    const pendingChangeKeyRef = useRef<string | null | undefined>(changeKey);
    const changePendingRef = useRef(false);
    const composingRef = useRef(false);
    const hasContentRef = useRef(false);
    const serializePlainTextRef = useRef(testOnlySerializePlainText ?? serializePlainText);
    const changeDelayMsRef = useRef(testOnlyChangeDelayMs ?? COMPOSER_CHANGE_DELAY_MS);
    const changeKeyRef = useRef(changeKey);
    const flushPendingChangeRef = useRef<
      (options?: { changeKey?: string | null; persistWithoutRender?: boolean }) => boolean
    >(() => true);
    serializePlainTextRef.current = testOnlySerializePlainText ?? serializePlainText;
    changeDelayMsRef.current = testOnlyChangeDelayMs ?? COMPOSER_CHANGE_DELAY_MS;
    changeKeyRef.current = changeKey;
    // Latest callbacks behind refs so the editor (created once) never closes
    // over a stale handler.
    const onChangeRef = useRef(onChange);
    const onPendingChangePersistRef = useRef(onPendingChangePersist);
    const onSubmitRef = useRef(onSubmit);
    const onFocusChangeRef = useRef(onFocusChange);
    const onContentChangeRef = useRef(onContentChange);
    const onBuiltinSlashCommandRef = useRef(onBuiltinSlashCommand);
    const hiddenBuiltinSlashCommandsRef = useRef(hiddenBuiltinSlashCommands);
    const onReadyRef = useRef(onReady);
    useEffect(() => {
      onChangeRef.current = onChange;
      onPendingChangePersistRef.current = onPendingChangePersist;
      onSubmitRef.current = onSubmit;
      onFocusChangeRef.current = onFocusChange;
      onContentChangeRef.current = onContentChange;
      onBuiltinSlashCommandRef.current = onBuiltinSlashCommand;
      hiddenBuiltinSlashCommandsRef.current = hiddenBuiltinSlashCommands;
      onReadyRef.current = onReady;
      skillsRef.current = skills;
    }, [
      onChange,
      onPendingChangePersist,
      onSubmit,
      onFocusChange,
      onContentChange,
      onBuiltinSlashCommand,
      hiddenBuiltinSlashCommands,
      onReady,
      skills,
    ]);

    useEffect(() => {
      document.querySelectorAll(".agent-category-menu-host").forEach((host) => {
        host.dispatchEvent(new CustomEvent(CATEGORY_SKILLS_CHANGED_EVENT));
      });
    }, [skills]);

    function clearPendingChangeTimer() {
      if (pendingChangeTimerRef.current === null) return;
      window.clearTimeout(pendingChangeTimerRef.current);
      pendingChangeTimerRef.current = null;
    }

    function armPendingChangeTimer() {
      clearPendingChangeTimer();
      if (composingRef.current || !changePendingRef.current) return;
      pendingChangeTimerRef.current = window.setTimeout(() => {
        pendingChangeTimerRef.current = null;
        flushPendingChangeRef.current();
      }, changeDelayMsRef.current);
    }

    function scheduleChange(nextEditor: Editor) {
      pendingChangeEditorRef.current = nextEditor;
      pendingChangeKeyRef.current = changeKeyRef.current;
      changePendingRef.current = true;
      armPendingChangeTimer();
    }

    function flushPendingChange(options?: {
      changeKey?: string | null;
      persistWithoutRender?: boolean;
    }) {
      if (!changePendingRef.current) return true;
      const nextEditor = pendingChangeEditorRef.current;
      if (!editorHasView(nextEditor)) {
        clearPendingChangeTimer();
        changePendingRef.current = false;
        return true;
      }
      // ProseMirror can retain its composing flag briefly after the DOM's
      // compositionend event. Keep the trailing publish armed until both
      // layers agree that the document is final.
      if (composingRef.current || nextEditor.view.composing) {
        armPendingChangeTimer();
        return false;
      }
      clearPendingChangeTimer();
      changePendingRef.current = false;
      const publish = options?.persistWithoutRender
        ? onPendingChangePersistRef.current
        : onChangeRef.current;
      publish?.(
        serializePlainTextRef.current(nextEditor.state.doc),
        categoryFromDoc(nextEditor.state.doc),
        options && "changeKey" in options ? options.changeKey : pendingChangeKeyRef.current,
      );
      return true;
    }
    flushPendingChangeRef.current = flushPendingChange;

    useEffect(
      () => () => {
        // useEditor's cleanup only schedules destruction for a later task,
        // leaving the live document available for this cleanup to persist.
        const pendingEditor = pendingChangeEditorRef.current;
        if (
          changePendingRef.current &&
          editorHasView(pendingEditor) &&
          !composingRef.current &&
          !pendingEditor.view.composing
        ) {
          onPendingChangePersistRef.current?.(
            serializePlainTextRef.current(pendingEditor.state.doc),
            categoryFromDoc(pendingEditor.state.doc),
            pendingChangeKeyRef.current,
          );
        }
        if (pendingChangeTimerRef.current !== null) {
          window.clearTimeout(pendingChangeTimerRef.current);
          pendingChangeTimerRef.current = null;
        }
        pendingChangeEditorRef.current = null;
        changePendingRef.current = false;
      },
      [],
    );

    function updateScrollFades(nextEditor: Editor | null) {
      const frame = frameRef.current;
      if (!frame || !editorHasView(nextEditor)) return;
      const scroller = nextEditor.view.dom;
      // A range selection paints a full-width highlight, and the edge fades
      // shave it — which reads as the selection being clipped, not as content
      // melting out of view. Drop the fades until the selection collapses.
      if (!nextEditor.state.selection.empty) {
        delete frame.dataset.fadeTop;
        delete frame.dataset.fadeBottom;
        delete frame.dataset.caretEdge;
        return;
      }
      const selectionEdge = composerDocumentEdge(
        nextEditor.state.selection.head,
        nextEditor.state.doc.content.size,
      );
      if (selectionEdge) {
        const caret = nextEditor.view.coordsAtPos(nextEditor.state.selection.head);
        const box = scroller.getBoundingClientRect();
        const caretVisible = caret.bottom >= box.top && caret.top <= box.bottom;
        if (caretVisible) frame.dataset.caretEdge = selectionEdge;
        else delete frame.dataset.caretEdge;
      } else {
        delete frame.dataset.caretEdge;
      }
      const overflow = scroller.scrollHeight - scroller.clientHeight > 1;
      const atTop = scroller.scrollTop <= 1;
      const atBottom = scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 1;
      if (overflow && !atTop) {
        frame.dataset.fadeTop = "true";
      } else {
        delete frame.dataset.fadeTop;
      }
      if (overflow && !atBottom) {
        frame.dataset.fadeBottom = "true";
      } else {
        delete frame.dataset.fadeBottom;
      }
    }

    const editor = useEditor({
      onFocus: () => onFocusChangeRef.current?.(true),
      onBlur: () => {
        flushPendingChangeRef.current();
        onFocusChangeRef.current?.(false);
      },
      extensions: [
        StarterKit.configure({
          // Truly-plain composer: no block structure or inline marks, just text,
          // soft line breaks, undo/redo, and the gap cursor (which makes landing
          // beside the atom chip feel right). Everything else is off.
          heading: false,
          bulletList: false,
          orderedList: false,
          listItem: false,
          listKeymap: false,
          blockquote: false,
          codeBlock: false,
          horizontalRule: false,
          bold: false,
          italic: false,
          strike: false,
          code: false,
          underline: false,
          link: false,
          trailingNode: false,
        }),
        Placeholder.configure({ placeholder }),
        createCategoryChip({
          skills: () => skillsRef.current,
          onBuiltinCommand: (name) => {
            if (!flushPendingChange({ changeKey: changeKeyRef.current })) return false;
            return onBuiltinSlashCommandRef.current?.(name) ?? false;
          },
          hiddenBuiltinCommands: () => hiddenBuiltinSlashCommandsRef.current,
        }),
        createNoteReference(),
      ],
      editorProps: {
        attributes: () => ({
          class: "agent-composer-editor",
          role: "textbox",
          "aria-label": editorLabelRef.current,
          "aria-multiline": "true",
        }),
        handleScrollToSelection: (view) => {
          // ProseMirror's own scrollIntoView walks every scrollable ancestor,
          // and the composer is a DOM child of the chat scroller
          // (.agent-scroll) — so growing the draft (Shift+Enter, paste)
          // dragged the conversation behind the fixed box. Scroll only the
          // editor's own scroller and stop the walk.
          const scroller = view.dom;
          const caret = view.coordsAtPos(view.state.selection.head);
          const box = scroller.getBoundingClientRect();
          const frame = frameRef.current;
          const selectionEdge = composerDocumentEdge(
            view.state.selection.head,
            view.state.doc.content.size,
          );
          if (frame) {
            if (selectionEdge) frame.dataset.caretEdge = selectionEdge;
            else delete frame.dataset.caretEdge;
          }
          if (selectionEdge === "start") {
            scroller.scrollTop = 0;
            if (frame) delete frame.dataset.fadeTop;
            return true;
          }
          if (selectionEdge === "end") {
            scroller.scrollTop = scroller.scrollHeight;
            if (frame) delete frame.dataset.fadeBottom;
            return true;
          }
          const frameStyles = frame ? getComputedStyle(frame) : null;
          const scrollerStyles = getComputedStyle(scroller);
          const margin = composerScrollMargin(
            frameStyles?.getPropertyValue("--scroll-fade-size") ?? "",
            scrollerStyles.paddingBottom,
          );
          if (caret.top < box.top + margin) {
            scroller.scrollTop -= box.top + margin - caret.top;
          } else if (caret.bottom > box.bottom - margin) {
            scroller.scrollTop += caret.bottom - (box.bottom - margin);
          }
          return true;
        },
        handleKeyDown: (view, event) => {
          if (
            event.key !== "Enter" ||
            event.shiftKey ||
            event.isComposing ||
            composingRef.current ||
            view.composing
          ) {
            return false;
          }
          // Suggestion palettes own Enter while open (they commit the
          // highlighted row); only a closed palette submits the message.
          if (document.querySelector(".agent-category-menu-host")) return false;
          event.preventDefault();
          if (!flushPendingChange({ changeKey: changeKeyRef.current })) return true;
          onSubmitRef.current();
          return true;
        },
        handleDOMEvents: {
          compositionstart: () => {
            composingRef.current = true;
            clearPendingChangeTimer();
            return false;
          },
          compositionend: () => {
            composingRef.current = false;
            // onUpdate during composition marked the snapshot dirty without
            // reading it. The final input transaction may arrive just after
            // compositionend; either way this trailing timer is rescheduled
            // by that transaction and publishes only the final document.
            armPendingChangeTimer();
            return false;
          },
        },
      },
      onCreate: ({ editor }) => {
        pendingChangeEditorRef.current = editor;
        queueMicrotask(() => {
          updateScrollFades(editor);
          if (editorHasView(editor)) onReadyRef.current?.(editor);
        });
      },
      onUpdate: ({ editor }) => {
        const hasContent = hasSubmittableContent(editor.state.doc);
        if (hasContentRef.current !== hasContent) {
          hasContentRef.current = hasContent;
          onContentChangeRef.current?.(hasContent);
        }
        scheduleChange(editor);
        requestAnimationFrame(() => updateScrollFades(editor));
      },
      onSelectionUpdate: ({ editor }) => {
        requestAnimationFrame(() => updateScrollFades(editor));
      },
    });

    useEffect(() => {
      if (!editor) return;
      let detachScrollTracking: (() => void) | null = null;
      const attachScrollTracking = () => {
        if (detachScrollTracking || !editorHasView(editor)) return;
        const scroller = editor.view.dom;
        let frame = 0;
        const schedule = () => {
          window.cancelAnimationFrame(frame);
          frame = window.requestAnimationFrame(() => updateScrollFades(editor));
        };
        scroller.addEventListener("scroll", schedule, { passive: true });
        window.addEventListener("resize", schedule);
        const observer =
          typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
        observer?.observe(scroller);
        schedule();
        detachScrollTracking = () => {
          window.cancelAnimationFrame(frame);
          scroller.removeEventListener("scroll", schedule);
          window.removeEventListener("resize", schedule);
          observer?.disconnect();
        };
      };

      const handleViewReady = () => {
        attachScrollTracking();
        flushEditorActions(editor, pendingEditorActionsRef, pendingChangeKeyRef);
      };

      // Register first so a view created between the readiness check and the
      // subscription still gets its scroll tracking and queued updates.
      editor.on("mount", handleViewReady);
      editor.on("create", handleViewReady);
      handleViewReady();
      return () => {
        editor.off("mount", handleViewReady);
        editor.off("create", handleViewReady);
        detachScrollTracking?.();
      };
    }, [editor]);

    useEffect(() => {
      if (editorHasView(editor)) editor.view.dom.setAttribute("aria-label", editorLabel);
    }, [editor, editorLabel]);

    useImperativeHandle(ref, () => {
      const applyOrQueue = (action: PendingEditorAction) => {
        if (pendingEditorActionsRef.current.length === 0 && editorHasView(editor)) {
          applyEditorAction(editor, action, pendingChangeKeyRef);
          return;
        }
        pendingEditorActionsRef.current = queueEditorAction(
          pendingEditorActionsRef.current,
          action,
        );
        // The view can become available before React runs the readiness
        // effect. Flush the reconciled queue now so an older request can never
        // overwrite this newer one when the create event arrives.
        flushEditorActions(editor, pendingEditorActionsRef, pendingChangeKeyRef);
      };

      return {
        focus: () => applyOrQueue({ type: "focus" }),
        flushPendingChange: (options) => flushPendingChangeRef.current(options),
        clear: () => applyOrQueue({ type: "clear" }),
        setContent: (text, category, options) =>
          applyOrQueue({
            type: "setContent",
            text,
            category,
            options: options ? { ...options } : undefined,
          }),
        insertCategory: (category) => applyOrQueue({ type: "insertCategory", category }),
        insertNoteReference: (noteReference) =>
          applyOrQueue({
            type: "insertNoteReference",
            noteReference: { ...noteReference },
          }),
        insertPlainText: (text) => {
          if (!editorHasView(editor)) return false;
          const normalized = text.replace(/\r\n?/g, "\n");
          const content = normalized.split("\n").flatMap((line, index) => {
            const nodes = [];
            if (index > 0) nodes.push(editor.schema.nodes.hardBreak.create());
            if (line) nodes.push(editor.schema.text(line));
            return nodes;
          });
          const transaction = closeHistory(editor.state.tr).replaceSelection(
            new Slice(Fragment.fromArray(content), 0, 0),
          );
          if (!transaction.docChanged) return false;
          editor.view.dispatch(transaction);
          return true;
        },
        isFocused: () => editorHasView(editor) && editor.isFocused && document.hasFocus(),
        isEmpty: () => !editorHasView(editor) || editor.isEmpty,
      };
    }, [editor]);

    return (
      <div ref={frameRef} className="agent-composer-editor-root scroll-fade">
        <EditorContent editor={editor} />
      </div>
    );
  },
);
ComposerEditor.displayName = "ComposerEditor";
