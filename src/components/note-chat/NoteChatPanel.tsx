import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { IconArrowUp } from "central-icons/IconArrowUp";
import { IconArrowUpRight } from "central-icons/IconArrowUpRight";
import { IconChecklist } from "central-icons/IconChecklist";
import { IconCrossMedium } from "central-icons/IconCrossMedium";
import { IconEmail1Sparkle } from "central-icons/IconEmail1Sparkle";
import { IconFileSparkle } from "central-icons/IconFileSparkle";
import { IconFlag1 } from "central-icons/IconFlag1";
import { IconMicrophone } from "central-icons/IconMicrophone";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { IconStop } from "central-icons/IconStop";
import { useCallback, useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent, ReactNode } from "react";

import { t as translate, useT } from "../../i18n";
import {
  displayedComposerUserMessageText,
  type AgentChatPart,
  type AgentChatTurn,
} from "../../lib/agent-chat-runtime";
import { type ChatEngineCatalog, chatEngineCatalog } from "../../lib/chat-engine";
import { messageFromError } from "../../lib/errors";
import { useScrollFade } from "../../lib/use-scroll-fade";
import { dictationHelperCommand } from "../../lib/tauri";
import { MarkdownContent } from "../agent/MarkdownContent";
import { AgentAttachmentTile } from "../agent/composer/AgentAttachmentTile";
import { ChatEnginePicker } from "../agent/composer/ChatEnginePicker";
import { ComposerEditor, type ComposerEditorHandle } from "../agent/composer/ComposerEditor";
import { UpstreamProviderFailureNoticePart } from "../agent/chat-turns/RunNotices";
import type { NoteChat, NoteChatAttachment } from "./useNoteChat";

/** Note-tailored presets, shown as the main session view's preset chips (icon
 * + label). Like those, a click prefills the composer rather than sending —
 * the prompt lands in the box so the person sees exactly what will run before
 * starting a turn. The note reference is prepended by useNoteChat on the
 * first message, so these are just the questions. The prompt is sent to the
 * model, so it stays in English; only the chip label follows the interface
 * language (a getter, so it reads the locale at render time). */
type NotePreset = { key: string; icon: ReactNode; label: string; prompt: string };

const NOTE_PRESETS: NotePreset[] = [
  {
    key: "summary",
    icon: <IconFileSparkle size={16} />,
    get label() {
      return translate("notes.chat.preset.summary");
    },
    prompt: "Summarize this note.",
  },
  {
    key: "actions",
    icon: <IconChecklist size={16} />,
    get label() {
      return translate("notes.chat.preset.actions");
    },
    prompt: "List the action items from this note and who owns each.",
  },
  {
    key: "decisions",
    icon: <IconFlag1 size={16} />,
    get label() {
      return translate("notes.chat.preset.decisions");
    },
    prompt: "What decisions were made in this note?",
  },
  {
    key: "followup",
    icon: <IconEmail1Sparkle size={16} />,
    get label() {
      return translate("notes.chat.preset.followup");
    },
    prompt: "Draft a follow-up email summarizing this note and its action items.",
  },
];

const NOTE_CHAT_WIDTH_KEY = "clovy:note-chat:panel-width";
const NOTE_CHAT_MIN_W = 300;
const NOTE_CHAT_MAX_W = 600;

function clampNoteChatWidth(width: number) {
  const viewportCap =
    typeof window === "undefined" ? NOTE_CHAT_MAX_W : Math.round(window.innerWidth * 0.48);
  const max = Math.max(NOTE_CHAT_MIN_W, Math.min(NOTE_CHAT_MAX_W, viewportCap));
  return Math.min(Math.max(Math.round(width), NOTE_CHAT_MIN_W), max);
}

/** The first message of a note chat carries the note reference token so
 * Clovy resolves the note; the panel already says which note it's about, so
 * the token is chrome, not content, in the panel's own transcript. */
function stripLeadingNoteToken(text: string) {
  return text.replace(/^@note:[\w-]+(?: \("[^"]*"\))?\s*/, "");
}

function userTurnText(turn: AgentChatTurn) {
  return turn.parts
    .map((part) =>
      part.type === "text"
        ? displayedComposerUserMessageText(stripLeadingNoteToken(part.text))
        : "",
    )
    .filter(Boolean)
    .join("\n");
}

function assistantPartNode(part: AgentChatPart, index: number, onRetry?: () => void) {
  switch (part.type) {
    case "text":
      return <MarkdownContent key={index} markdown={part.text} repairProse />;
    case "notice":
      if (part.kind === "upstream-provider") {
        return <UpstreamProviderFailureNoticePart key={index} onRetry={onRetry} />;
      }
      return <MarkdownContent key={index} markdown={part.text} />;
    case "tool":
      return (
        <div
          key={index}
          className="note-chat-tool"
          data-running={part.status === "running" || undefined}
        >
          {part.name}
        </div>
      );
    default:
      // Reasoning, approvals, images and the other rich parts stay in the
      // full agent view; the panel keeps to prose + tool beats and offers
      // "Open in agent view" for everything else.
      return null;
  }
}

/** The contextual Ask Clovy chat: a fixed side panel next to the meeting note,
 * mirroring the agent artifact panel's attach mechanics (sibling card on the
 * window background; the main card slides left via the :has() margin in
 * app.css). The conversation is a real agent session scoped to the
 * note — see useNoteChat. */
export function NoteChatPanel({
  note,
  chat,
  recordingActive,
  onClose,
  onOpenInAgent,
}: {
  note: { id: string; title: string };
  /** The note's chat session, owned by App so it keeps running when the panel
   * closes (background work + toolbar working dot). See useNoteChat. */
  chat: NoteChat;
  /** True while a meeting/dictation recording is capturing. The Rust side
   * ducks the recording's mic while dictation listens (so a dictated question
   * never lands in the note); this only tunes the tooltip to say so. */
  recordingActive?: boolean;
  onClose: () => void;
  onOpenInAgent: (sessionId: string | undefined) => void;
}) {
  const t = useT();
  const {
    turns,
    working,
    loading,
    error,
    storedSessionId,
    model: activeModelId,
    setModel,
    submit,
    retry,
    stop,
  } = chat;
  // Block escalation only during the pure first-send race — the session is
  // being created and there's no id to hand off yet. Once an id exists the
  // agent view resolves the conversation by it, so opening is always safe.
  const escalationPending = working && !storedSessionId;
  const [entered, setEntered] = useState(false);
  const [draftEmpty, setDraftEmpty] = useState(true);
  const [attachments, setAttachments] = useState<NoteChatAttachment[]>([]);
  const [composerError, setComposerError] = useState<string | null>(null);
  const composerRef = useRef<ComposerEditorHandle | null>(null);
  const draftRef = useRef("");
  const panelRef = useRef<HTMLElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  // The runtime receives the selected local paths and imports them into the
  // session workspace before the run begins.
  async function pickAttachments() {
    try {
      const selected = await openFileDialog({ multiple: true, title: t("notes.chat.attachFiles") });
      if (!selected) return;
      const paths = (Array.isArray(selected) ? selected : [selected])
        .map((path) => path.trim())
        .filter(Boolean)
        .slice(0, 8);
      if (!paths.length) return;
      setComposerError(null);
      setAttachments((current) => [
        ...current,
        ...paths.map((path) => ({
          id: `${path}:${Date.now()}:${Math.random().toString(36)}`,
          name: path.split(/[\\/]/).at(-1) || path,
          path,
        })),
      ]);
    } catch (err) {
      setComposerError(messageFromError(err));
    }
  }

  // Focus the composer, then explicitly start the dictation helper. Stop and
  // discard controls are correlated separately with the take shown in the HUD.
  async function startDictation() {
    composerRef.current?.focus();
    try {
      await dictationHelperCommand({ type: "start_listening" });
    } catch (err) {
      setComposerError(messageFromError(err));
    }
  }

  // Model picking shares the regular agent catalog. The runtime receives the
  // selected model with each run, so the panel can change it without a legacy
  // transport-side configuration step.
  const [engineCatalog, setEngineCatalog] = useState<ChatEngineCatalog | null>(null);
  const footerRef = useRef<HTMLElement>(null);
  useEffect(() => {
    let stale = false;
    void chatEngineCatalog()
      .then((catalog) => {
        if (!stale) setEngineCatalog(catalog);
      })
      .catch(() => undefined);
    return () => {
      stale = true;
    };
  }, []);
  function selectModel(nextModelId: string) {
    setModel(nextModelId);
    setComposerError(null);
  }

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      // The engine menu consumes Escape first. A
      // draft in progress blocks the final close — it must not eat the words.
      if (!draftEmpty) return;
      onCloseRef.current();
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [draftEmpty]);

  // Restore the remembered width once per mount; the var lives on .app-shell
  // because the main card's margin and the editor footer consume it too.
  useEffect(() => {
    const shell = panelRef.current?.closest(".app-shell");
    if (!(shell instanceof HTMLElement)) return;
    const stored = Number.parseInt(window.localStorage.getItem(NOTE_CHAT_WIDTH_KEY) ?? "", 10);
    if (Number.isFinite(stored)) {
      shell.style.setProperty("--note-chat-w", `${clampNoteChatWidth(stored)}px`);
    }
  }, []);

  // Drag-resize from the panel's left edge, mirroring the files panel: the
  // var tracks the cursor with transitions suppressed (data-note-chat-resizing)
  // and the final width persists on release.
  const startResize = useCallback((event: ReactPointerEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const shell = event.currentTarget.closest(".app-shell");
    const startWidth = panelRef.current?.offsetWidth;
    if (!(shell instanceof HTMLElement) || !startWidth) return;
    shell.setAttribute("data-note-chat-resizing", "true");
    const startX = event.clientX;
    const onMove = (move: PointerEvent) => {
      const next = clampNoteChatWidth(startWidth + (startX - move.clientX));
      shell.style.setProperty("--note-chat-w", `${next}px`);
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      shell.removeAttribute("data-note-chat-resizing");
      const finalWidth = panelRef.current?.offsetWidth;
      if (finalWidth) {
        window.localStorage.setItem(NOTE_CHAT_WIDTH_KEY, `${finalWidth}`);
      }
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp, { once: true });
  }, []);

  // Edge fades on the thread (same trick as the files panel): the bar has no
  // divider, so the top fade is what signals content scrolled up behind it.
  const scrollerRef = useRef<HTMLDivElement>(null);
  const fade = useScrollFade(scrollerRef);

  // Keep the newest turn in view as the conversation grows or streams.
  const lastTurn = turns.at(-1);
  const lastTurnSize = lastTurn?.parts.reduce(
    (size, part) => size + ("text" in part ? part.text.length : 0),
    0,
  );
  useEffect(() => {
    const scroller = scrollerRef.current;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
    fade.update();
  }, [turns.length, lastTurnSize, working, fade.update]);

  async function handleSend() {
    if (working) return;
    if (!composerRef.current?.flushPendingChange()) return;
    const serializedText = draftRef.current;
    const text = serializedText.trim();
    setComposerError(null);
    const result = await submit(text, attachments);
    // Clear the composer only when this panel still owns the accepted send.
    // A stale/switched panel must not wipe the draft of the newly selected note.
    if (result.accepted && result.current) {
      if (!composerRef.current?.flushPendingChange()) return;
      if (draftRef.current !== serializedText) return;
      draftRef.current = "";
      setDraftEmpty(true);
      setAttachments([]);
      composerRef.current?.clear();
      composerRef.current?.focus();
    }
  }

  // A preset drops its prompt into the composer for the person to send or
  // edit — never auto-submits — matching the main session view's presets so a
  // click always lands in the box before it costs anything.
  function prefillPreset(preset: NotePreset) {
    draftRef.current = preset.prompt;
    setDraftEmpty(false);
    composerRef.current?.setContent(preset.prompt, null, { focus: true });
  }

  const streamingVisibly =
    lastTurn?.role === "assistant" &&
    lastTurn.status === "running" &&
    lastTurn.parts.some((part) => part.type === "text" && part.text.trim());
  const runningTool =
    lastTurn?.status === "running"
      ? [...lastTurn.parts]
          .reverse()
          .find((part) => part.type === "tool" && part.status === "running")
      : undefined;

  return (
    <>
      <div
        className="note-chat-resize-handle"
        role="separator"
        aria-orientation="vertical"
        aria-label={t("notes.chat.resize")}
        onPointerDown={startResize}
      />
      <aside
        ref={panelRef}
        className="note-chat-panel"
        aria-label={t("notes.chat.panelLabel")}
        data-entered={entered || undefined}
        onAnimationEnd={(event) => {
          if (event.animationName === "note-chat-panel-in") setEntered(true);
        }}
      >
        <header className="note-chat-bar">
          <h2 className="note-chat-bar-title">{t("notes.header.ask")}</h2>
          <button
            type="button"
            className="icon-button"
            aria-label={t("notes.chat.openInAgent")}
            title={escalationPending ? t("notes.chat.finishingUp") : t("notes.chat.openInAgent")}
            disabled={escalationPending}
            onClick={() => onOpenInAgent(storedSessionId)}
          >
            <IconArrowUpRight size={15} />
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label={t("notes.chat.close")}
            title={t("common.close")}
            onClick={onClose}
          >
            <IconCrossMedium size={15} />
          </button>
        </header>
        <div ref={scrollerRef} className="note-chat-scroll scroll-fade-mask" {...fade.props}>
          {turns.length === 0 && !loading ? (
            <div className="note-chat-empty">
              <p className="note-chat-empty-lead">{t("notes.chat.emptyLead")}</p>
              <div className="note-chat-suggestions">
                {NOTE_PRESETS.map((preset) => (
                  <button
                    key={preset.key}
                    type="button"
                    className="agent-hero-chip"
                    disabled={working}
                    onClick={() => prefillPreset(preset)}
                  >
                    <span className="agent-hero-chip-icon" aria-hidden>
                      {preset.icon}
                    </span>
                    {preset.label}
                  </button>
                ))}
              </div>
            </div>
          ) : (
            <div className="note-chat-thread">
              {turns.map((turn) =>
                turn.role === "user" ? (
                  <div key={turn.id} className="note-chat-turn-user">
                    {userTurnText(turn)}
                  </div>
                ) : (
                  <div key={turn.id} className="note-chat-turn-assistant">
                    {turn.parts.map((part, index) =>
                      assistantPartNode(
                        part,
                        index,
                        working || chat.submissionPending ? undefined : () => void retry(turn.id),
                      ),
                    )}
                  </div>
                ),
              )}
              {working && !streamingVisibly ? (
                <div className="note-chat-working">
                  {runningTool && "name" in runningTool
                    ? runningTool.name
                    : t("notes.chat.thinking")}
                </div>
              ) : null}
            </div>
          )}
          {error || composerError ? (
            <div className="note-chat-error" role="alert">
              {error ?? composerError}
            </div>
          ) : null}
        </div>
        <footer ref={footerRef} className="note-chat-composer">
          {/* The actual chatbox: the agent composer's box/attach/toolbar/model/
           * send classes, wired to the panel's session. */}
          <div className="agent-composer-box">
            {attachments.length ? (
              <div className="agent-composer-attachments">
                {attachments.map((attachment) => (
                  <AgentAttachmentTile
                    key={attachment.id}
                    name={attachment.name}
                    onRemove={() =>
                      setAttachments((current) =>
                        current.filter((item) => item.id !== attachment.id),
                      )
                    }
                  />
                ))}
              </div>
            ) : null}
            <ComposerEditor
              ref={composerRef}
              placeholder={t("notes.chat.placeholder")}
              onChange={(text) => {
                draftRef.current = text;
                setDraftEmpty(!text.trim());
              }}
              onContentChange={(hasContent) => setDraftEmpty(!hasContent)}
              onSubmit={() => void handleSend()}
            />
            <div className="agent-composer-toolbar">
              <button
                type="button"
                className="agent-composer-attach"
                aria-label={t("notes.chat.attachFiles")}
                title={t("notes.chat.attachFiles")}
                onClick={() => void pickAttachments()}
              >
                <IconPlusMedium size={18} />
              </button>
              <div className="agent-composer-actions">
                <ChatEnginePicker
                  model={activeModelId}
                  setModel={selectModel}
                  catalog={engineCatalog}
                  anchorRef={footerRef}
                />
                <button
                  type="button"
                  className="agent-composer-mic"
                  aria-label={t("notes.chat.dictate")}
                  title={
                    recordingActive
                      ? t("notes.chat.dictateDuringRecording")
                      : t("notes.chat.startDictation")
                  }
                  onClick={() => void startDictation()}
                >
                  <IconMicrophone size={18} />
                </button>
                {working ? (
                  <button
                    type="button"
                    className="agent-composer-stop"
                    aria-label={t("notes.chat.stop")}
                    title={t("notes.chat.stop")}
                    onClick={stop}
                  >
                    <IconStop size={16} />
                  </button>
                ) : (
                  <button
                    type="button"
                    className="agent-composer-send"
                    aria-label={t("notes.chat.send")}
                    disabled={draftEmpty && !attachments.length}
                    onClick={() => void handleSend()}
                  >
                    <IconArrowUp size={18} />
                  </button>
                )}
              </div>
            </div>
          </div>
        </footer>
      </aside>
    </>
  );
}
