import { listen } from "@tauri-apps/api/event";
import { useClovyDataChanged } from "../../lib/clovy-data-events";
import { IconChevronRightSmall } from "central-icons/IconChevronRightSmall";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconFontStyle } from "central-icons/IconFontStyle";
import { IconInfinity } from "central-icons/IconInfinity";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { IconMicrophone } from "central-icons/IconMicrophone";
import { IconMicrophoneSparkle } from "central-icons/IconMicrophoneSparkle";
import { IconMicrophoneSparkle as IconMicrophoneSparkleFilled } from "central-icons-filled/IconMicrophoneSparkle";
import { IconSpeachToText } from "central-icons/IconSpeachToText";
import { IconTrashCanSimple } from "central-icons/IconTrashCanSimple";
import {
  type KeyboardEvent as ReactKeyboardEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { clockTimeOptions, formatDate, t as translate, useLocale, useT } from "../../i18n";
import { isSameLocalDay } from "../../lib/date-format";
import { AppIcon } from "../activity-timeline/AppIcon";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { CopyStateIcon } from "../ui/CopyStateIcon";
import { Dialog } from "../ui/Dialog";
import { EmptyState } from "../ui/EmptyState";
import { HoverTip } from "../ui/HoverTip";
import { KeycapShortcut } from "../shortcuts/KeycapShortcut";
import {
  deleteDictationHistoryItem,
  dictationSettings,
  listDictationHistory,
  listDictionaryEntries,
  type DictationSettingsDto,
  type DictationHistoryItemDto,
} from "../../lib/tauri";
import { parseDictationHelperEvent } from "../../lib/dictation-events";
import { useForcedEmptyStates } from "../../lib/empty-states-demo";
import { useDictationCapabilities } from "../../lib/platform";
import { useScrollFade } from "../../lib/use-scroll-fade";

const NO_DICTATIONS: DictationHistoryItemDto[] = [];

/** Which Settings section a "Set up" link drives to. */
export type DictationSettingsTarget = "style" | "dictionary";

type DictationHistoryViewProps = {
  onNavigateToSettings?: (target: DictationSettingsTarget) => void;
};

type HistoryGroup = {
  label: string;
  items: DictationHistoryItemDto[];
};

// Persists the dismissed state of the "Get more from dictation" card. The card
// points at two *optional* power features (writing styles, personal
// dictionary), so a dismiss is honored permanently — we don't badger people
// about extras they've chosen to skip.
const HINT_DISMISSED_KEY = "os-clovy:dictation-hint-dismissed";

// The card only surfaces once someone has clearly adopted dictation, so we
// never stack setup suggestions on a newcomer who's still learning the gesture.
// Note: history is the last 7 days (capped at 200), so this is a recent-usage
// signal, not a lifetime count.
const MIN_DICTATIONS_FOR_HINT = 10;

// The default writing style; anything else means the user has configured one.
const DEFAULT_STYLE = "standard";

function readHintDismissed() {
  try {
    return localStorage.getItem(HINT_DISMISSED_KEY) === "1";
  } catch {
    return false;
  }
}

export function DictationHistoryView({ onNavigateToSettings }: DictationHistoryViewProps = {}) {
  const t = useT();
  const locale = useLocale();
  const [allItems, setItems] = useState<DictationHistoryItemDto[]>([]);
  const [retentionDays, setRetentionDays] = useState(7);
  const [query, setQuery] = useState("");
  const [loadingState, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // __emptyStates() preview (dev console): render the page as a fresh
  // install would see it, real data untouched underneath.
  const forcedEmpty = useForcedEmptyStates();
  const items = forcedEmpty ? NO_DICTATIONS : allItems;
  const loading = !forcedEmpty && loadingState;
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const copyResetTimerRef = useRef<number | undefined>(undefined);
  const [settings, setSettings] = useState<DictationSettingsDto>();
  const [dictionaryCount, setDictionaryCount] = useState<number | null>(null);
  const [hintDismissed, setHintDismissed] = useState(readHintDismissed);
  const [pendingDelete, setPendingDelete] = useState<DictationHistoryItemDto | null>(null);
  const capabilities = useDictationCapabilities();
  const dictationAvailable = capabilities.available;
  useClovyDataChanged("dictations", async (_change, isCurrent) => {
    try {
      const response = await listDictationHistory();
      if (!isCurrent()) return;
      setItems(response.items);
      setRetentionDays(response.retentionDays);
      setError(null);
    } catch (caught) {
      if (isCurrent()) setError(messageFromError(caught));
    }
  });

  const loadHistory = useCallback(async () => {
    try {
      const response = await listDictationHistory();
      setItems(response.items);
      setRetentionDays(response.retentionDays);
      setError(null);
    } catch (err) {
      setError(messageFromError(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadHistory();
  }, [loadHistory]);

  useEffect(() => {
    dictationSettings()
      .then((response) => setSettings(response.settings))
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    listDictionaryEntries()
      .then((entries) => setDictionaryCount(entries.length))
      .catch(() => setDictionaryCount(0));
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen<string>("dictation-event", (event) => {
      const payload = parseDictationHelperEvent(event.payload);
      if (payload?.type === "final_transcript") {
        void loadHistory();
      }
    }).then((cleanup) => {
      // Unmount can race the listen() promise — unsubscribe immediately
      // instead of leaking the listener.
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [loadHistory]);

  useEffect(
    () => () => {
      if (copyResetTimerRef.current !== undefined) {
        window.clearTimeout(copyResetTimerRef.current);
      }
    },
    [],
  );

  const filtered = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return items;
    return items.filter((item) =>
      `${item.text} ${item.provider} ${item.language ?? ""}`.toLowerCase().includes(normalized),
    );
  }, [items, query]);

  // Group labels ("Today", weekday dates) follow the interface language.
  // biome-ignore lint/correctness/useExhaustiveDependencies: locale re-derives the labels
  const groups = useMemo(() => groupHistoryItems(filtered), [filtered, locale]);

  const defaultPushToTalk = capabilities.platform === "macos" ? "Ctrl+Opt+D" : "Ctrl+Alt+D";
  const defaultToggle = capabilities.platform === "macos" ? "Ctrl+Opt+T" : "Ctrl+Alt+T";
  const pushToTalk = settings?.pushToTalkShortcut.label ?? defaultPushToTalk;
  const toggle = settings?.toggleShortcut.label ?? defaultToggle;

  // Show each optional feature only while it's still unconfigured, and only
  // once we know its state (avoids the card flashing in then vanishing). The
  // card itself appears only for adopted users who haven't dismissed it.
  const customizationLoaded = settings !== undefined && dictionaryCount !== null;
  const styleUnconfigured = settings?.style === DEFAULT_STYLE;
  const dictionaryUnconfigured = (dictionaryCount ?? 0) === 0;
  const showHint =
    !hintDismissed &&
    customizationLoaded &&
    items.length >= MIN_DICTATIONS_FOR_HINT &&
    (styleUnconfigured || dictionaryUnconfigured);

  function dismissHint() {
    setHintDismissed(true);
    try {
      localStorage.setItem(HINT_DISMISSED_KEY, "1");
    } catch {
      // best-effort; the card simply reappears next launch.
    }
  }

  async function copyDictation(item: DictationHistoryItemDto) {
    const text = item.text.trim();
    if (!text) return;
    await navigator.clipboard.writeText(`${text} `);
    setCopiedId(item.id);
    if (copyResetTimerRef.current !== undefined) {
      window.clearTimeout(copyResetTimerRef.current);
    }
    copyResetTimerRef.current = window.setTimeout(() => {
      setCopiedId(null);
      copyResetTimerRef.current = undefined;
    }, 1600);
  }

  async function confirmDelete() {
    const item = pendingDelete;
    if (!item) return;
    // Let errors propagate so ConfirmDialog keeps itself open on failure.
    await deleteDictationHistoryItem(item.id);
    setItems((prev) => prev.filter((entry) => entry.id !== item.id));
  }

  return (
    <section className="dictation-history-workspace" aria-label={t("recorder.dictation.title")}>
      <header className="folders-header">
        <div className="folders-heading">
          <h1>
            {t("recorder.dictation.title")}
            {items.length > 0 ? <span className="folders-count">{items.length}</span> : null}
          </h1>
          <p className="folders-subtitle">
            {t("recorder.dictation.subtitle", { count: retentionDays })}
          </p>
        </div>
        {/* Shortcuts live in the header whenever there's history; newcomers
            get them in the empty state instead. */}
        {items.length > 0 && dictationAvailable ? (
          <ShortcutLegend className="dictation-shortcuts" pushToTalk={pushToTalk} toggle={toggle} />
        ) : null}
      </header>

      {showHint ? (
        <GetMoreCard
          showStyles={styleUnconfigured}
          showDictionary={dictionaryUnconfigured}
          onDismiss={dismissHint}
          onSetUpStyles={() => onNavigateToSettings?.("style")}
          onSetUpDictionary={() => onNavigateToSettings?.("dictionary")}
        />
      ) : null}

      {items.length > 0 ? (
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

      {error ? <p className="error-banner">{error}</p> : null}

      {loading ? (
        <div className="folders-empty">
          <p>{t("recorder.dictation.loading")}</p>
        </div>
      ) : items.length === 0 ? (
        <EmptyState
          label={
            dictationAvailable
              ? t("recorder.dictation.empty.label")
              : t("recorder.dictation.unavailable.label")
          }
          icon={<IconMicrophoneSparkleFilled size={28} />}
          title={
            dictationAvailable
              ? t("recorder.dictation.empty.title")
              : t("recorder.dictation.unavailable.title")
          }
          description={
            dictationAvailable
              ? t("recorder.dictation.empty.description")
              : t("recorder.dictation.unavailable.description")
          }
          footer={
            dictationAvailable ? (
              <ShortcutLegend
                className="shortcut-legend-inline"
                pushToTalk={pushToTalk}
                toggle={toggle}
              />
            ) : null
          }
        />
      ) : groups.length === 0 ? (
        <div className="folders-empty">
          <p>{t("recorder.dictation.noMatches", { query: query.trim() })}</p>
        </div>
      ) : (
        <div className="dictation-history-groups">
          {groups.map((group) => (
            <section className="dictation-history-group" key={group.label}>
              <h2>{group.label}</h2>
              <ul className="dictation-history-list" role="list">
                {group.items.map((item) => (
                  <DictationHistoryRow
                    key={item.id}
                    item={item}
                    copied={copiedId === item.id}
                    onCopy={() => void copyDictation(item)}
                    onDelete={() => setPendingDelete(item)}
                  />
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}

      <ConfirmDialog
        open={pendingDelete !== null}
        onClose={() => setPendingDelete(null)}
        onConfirm={confirmDelete}
        title={t("recorder.dictation.delete.title")}
        description={t("recorder.dictation.delete.description")}
        confirmLabel={t("common.delete")}
        destructive
      />
    </section>
  );
}

/** A single transcription row: icon, transcript (clamped to two lines), time,
 * and copy/delete actions. When the transcript is clipped, clicking it opens a
 * dialog with the full, scrollable text — room to grow search/highlight later. */
function DictationHistoryRow({
  item,
  copied,
  onCopy,
  onDelete,
}: {
  item: DictationHistoryItemDto;
  copied: boolean;
  onCopy: () => void;
  onDelete: () => void;
}) {
  const t = useT();
  const textRef = useRef<HTMLParagraphElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [truncated, setTruncated] = useState(false);
  const [open, setOpen] = useState(false);
  // Position-aware scroll fades: only when the body actually overflows, and
  // only on the edge(s) with hidden content.
  const fade = useScrollFade(scrollRef);

  useEffect(() => {
    const el = textRef.current;
    if (!el) return;
    const measure = () => setTruncated(el.scrollHeight - el.clientHeight > 1);
    measure();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", measure);
      return () => window.removeEventListener("resize", measure);
    }
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [item.text]);

  useEffect(() => {
    if (!open) return;
    const id = requestAnimationFrame(fade.update);
    return () => cancelAnimationFrame(id);
  }, [open, fade.update]);

  const expandProps = truncated
    ? {
        role: "button" as const,
        tabIndex: 0,
        onClick: () => setOpen(true),
        onKeyDown: (event: ReactKeyboardEvent) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            setOpen(true);
          }
        },
      }
    : {};

  return (
    <li className="dictation-history-item" data-truncated={truncated || undefined}>
      <span className="dictation-history-icon" aria-hidden>
        <AppIcon
          bundleId={item.appBundleId ?? null}
          size={20}
          fallback={<IconMicrophoneSparkle size={14} />}
        />
      </span>
      <div className="dictation-history-body">
        <p
          ref={textRef}
          className="dictation-history-text"
          aria-label={truncated ? t("recorder.dictation.showFullTranscript") : undefined}
          {...expandProps}
        >
          {item.text}
        </p>
        {item.language ? <span className="dictation-history-lang">{item.language}</span> : null}
      </div>
      <time
        className="dictation-history-time"
        dateTime={item.createdAt}
        title={formatTranscriptTimestamp(item.createdAt)}
      >
        {formatTime(item.createdAt)}
      </time>
      <span className="dictation-history-actions">
        <HoverTip
          compact
          width={104}
          tip={copied ? t("common.copied") : t("common.copy")}
          forceOpen={copied && !open}
        >
          <button
            type="button"
            className="dictation-row-act"
            data-copied={copied}
            aria-label={copied ? t("common.copied") : t("common.copy")}
            onClick={onCopy}
          >
            <CopyStateIcon copied={copied} />
          </button>
        </HoverTip>
        <button
          type="button"
          className="dictation-row-act dictation-row-act-danger"
          aria-label={t("common.delete")}
          onClick={onDelete}
        >
          <IconTrashCanSimple size={14} />
        </button>
      </span>

      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        leading={
          <AppIcon
            bundleId={item.appBundleId ?? null}
            size={20}
            fallback={<IconMicrophoneSparkle size={15} />}
          />
        }
        title={formatTranscriptTimestamp(item.createdAt)}
        width={540}
        className="transcript-dialog"
        footer={
          <HoverTip
            compact
            width={104}
            tip={copied ? t("common.copied") : t("common.copy")}
            forceOpen={copied && open}
          >
            <button
              type="button"
              className="btn btn-secondary"
              aria-label={copied ? t("common.copied") : t("common.copy")}
              onClick={onCopy}
            >
              <CopyStateIcon copied={copied} />
              {t("common.copy")}
            </button>
          </HoverTip>
        }
      >
        <div className="transcript-dialog-scroll scroll-fade-mask" ref={scrollRef} {...fade.props}>
          <p>{item.text}</p>
        </div>
      </Dialog>
    </li>
  );
}

/** The "Get more from dictation" card. Surfaces only the optional features the
 * user hasn't set up yet; once both are configured it stops rendering entirely.
 * Shortcuts live in the header/empty state, not here. */
function GetMoreCard({
  showStyles,
  showDictionary,
  onDismiss,
  onSetUpStyles,
  onSetUpDictionary,
}: {
  showStyles: boolean;
  showDictionary: boolean;
  onDismiss: () => void;
  onSetUpStyles: () => void;
  onSetUpDictionary: () => void;
}) {
  const t = useT();
  return (
    <section className="dictation-hint" aria-label={t("recorder.dictation.hint.title")}>
      <button
        type="button"
        className="dictation-hint-dismiss"
        aria-label={t("recorder.dictation.hint.dismiss")}
        onClick={onDismiss}
      >
        <IconCrossSmall size={14} />
      </button>
      <h2 className="dictation-hint-title">{t("recorder.dictation.hint.title")}</h2>
      <div className="dictation-hint-items">
        {showStyles ? (
          <button type="button" className="dictation-hint-item" onClick={onSetUpStyles}>
            <span className="dictation-hint-chip" aria-hidden>
              <IconFontStyle size={16} />
            </span>
            <span className="dictation-hint-item-body">
              <span className="dictation-hint-item-name">
                {t("recorder.dictation.hint.style.name")}
              </span>
              <span className="dictation-hint-item-desc">
                {t("recorder.dictation.hint.style.description")}
              </span>
            </span>
            <span className="dictation-hint-setup">
              {t("recorder.dictation.hint.setUp")} <IconChevronRightSmall size={15} />
            </span>
          </button>
        ) : null}
        {showDictionary ? (
          <button type="button" className="dictation-hint-item" onClick={onSetUpDictionary}>
            <span className="dictation-hint-chip" aria-hidden>
              <IconSpeachToText size={16} />
            </span>
            <span className="dictation-hint-item-body">
              <span className="dictation-hint-item-name">
                {t("recorder.dictation.hint.dictionary.name")}
              </span>
              <span className="dictation-hint-item-desc">
                {t("recorder.dictation.hint.dictionary.description")}
              </span>
            </span>
            <span className="dictation-hint-setup">
              {t("recorder.dictation.hint.setUp")} <IconChevronRightSmall size={15} />
            </span>
          </button>
        ) : null}
      </div>
    </section>
  );
}

/** Icon + label + keycaps for each dictation shortcut. Renders stacked in the
 * header (after dismiss) or inline inside the hint card. */
function ShortcutLegend({
  className,
  pushToTalk,
  toggle,
}: {
  className: string;
  pushToTalk: string;
  toggle: string;
}) {
  const t = useT();
  return (
    <dl className={className} aria-label={t("recorder.dictation.shortcuts.aria")}>
      <div className="dictation-shortcut">
        <span className="dictation-shortcut-icon" aria-hidden>
          <IconMicrophone size={15} />
        </span>
        <dt>{t("recorder.dictation.shortcuts.pushToTalk")}</dt>
        <dd>
          <KeycapShortcut label={pushToTalk} />
        </dd>
      </div>
      <div className="dictation-shortcut">
        <span className="dictation-shortcut-icon" aria-hidden>
          <IconInfinity size={15} />
        </span>
        <dt>{t("recorder.dictation.shortcuts.handsFree")}</dt>
        <dd>
          <KeycapShortcut label={toggle} />
        </dd>
      </div>
    </dl>
  );
}

function groupHistoryItems(items: DictationHistoryItemDto[]): HistoryGroup[] {
  const groups: HistoryGroup[] = [];
  for (const item of items) {
    const label = formatGroupLabel(item.createdAt);
    const group = groups.find((candidate) => candidate.label === label);
    if (group) {
      group.items.push(item);
    } else {
      groups.push({ label, items: [item] });
    }
  }
  return groups;
}

function formatGroupLabel(iso: string) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return translate("recorder.dictation.earlier");
  const now = new Date();
  if (isSameLocalDay(date, now)) return translate("common.today");
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (isSameLocalDay(date, yesterday)) return translate("common.yesterday");
  return formatDate(date, {
    weekday: "long",
    month: "short",
    day: "numeric",
  });
}

function formatTime(iso: string) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return formatDate(date, {
    ...clockTimeOptions(),
  });
}

function formatTranscriptTimestamp(iso: string) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return formatDate(date, {
    weekday: "long",
    month: "short",
    day: "numeric",
    ...clockTimeOptions(),
  });
}

function messageFromError(err: unknown) {
  if (err && typeof err === "object" && "message" in err) {
    const message = (err as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  return translate("recorder.dictation.unavailableError");
}
