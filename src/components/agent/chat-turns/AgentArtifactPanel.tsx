import { IconArrowDown } from "central-icons/IconArrowDown";
import { IconArrowInbox } from "central-icons/IconArrowInbox";
import { IconArrowUp } from "central-icons/IconArrowUp";
import { IconChevronLeftSmall } from "central-icons/IconChevronLeftSmall";
import { IconCrossMedium } from "central-icons/IconCrossMedium";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import {
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useT } from "../../../i18n";
import { agentFilePreview, agentFileText } from "../../../lib/tauri";
import { useScrollFade } from "../../../lib/use-scroll-fade";
import { SegmentedControl } from "../../ui/SegmentedControl";
import { Spinner } from "../../ui/Spinner";
import { formatBytes } from "../agent-workspace-helpers";
import { fileTypeIconComponent } from "../FileTypeIcon";
import { MarkdownContent, highlightText, type HighlightCursor } from "../MarkdownContent";

export type AgentArtifact = {
  name: string;
  path: string;
  rootLabel: string;
  size?: number | null;
};

export type AgentArtifactPanelState = { view: "list" } | { view: "file"; artifact: AgentArtifact };

export function AgentArtifactList({
  artifacts,
  onDownload,
  onOpen,
}: {
  artifacts: AgentArtifact[];
  onDownload?: (artifact: AgentArtifact) => void;
  onOpen?: (artifact: AgentArtifact) => void;
}) {
  const t = useT();
  if (!artifacts.length) return null;
  return (
    <div className="agent-artifact-list" aria-label={t("chat.files.generatedFiles")}>
      {artifacts.map((artifact) => (
        <AgentArtifactCard
          key={artifact.path}
          artifact={artifact}
          onDownload={onDownload}
          onOpen={onOpen}
        />
      ))}
    </div>
  );
}

function AgentArtifactCard({
  artifact,
  onDownload,
  onOpen,
}: {
  artifact: AgentArtifact;
  onDownload?: (artifact: AgentArtifact) => void;
  onOpen?: (artifact: AgentArtifact) => void;
}) {
  const t = useT();
  const ArtifactIcon = fileTypeIconComponent(artifact.path);
  const summary = (
    <>
      <span className="agent-artifact-icon">
        <ArtifactIcon size={18} />
      </span>
      <div className="agent-artifact-meta">
        <span className="agent-artifact-name">{artifact.name}</span>
        {artifact.size != null ? (
          <span className="agent-artifact-size">{formatBytes(artifact.size)}</span>
        ) : null}
      </div>
    </>
  );

  return (
    <article className={`agent-artifact-card${onOpen ? " agent-artifact-card-interactive" : ""}`}>
      {onOpen ? (
        <button
          type="button"
          className="agent-artifact-open"
          aria-label={t("chat.openNamed", { name: artifact.name })}
          onClick={() => onOpen(artifact)}
        >
          {summary}
        </button>
      ) : (
        <div className="agent-artifact-open">{summary}</div>
      )}
      {onDownload ? (
        <button
          type="button"
          className="agent-artifact-download"
          aria-label={t("chat.files.downloadNamed", { name: artifact.name })}
          title={t("chat.media.download")}
          onClick={() => onDownload(artifact)}
        >
          <IconArrowInbox size={16} />
        </button>
      ) : null}
    </article>
  );
}

/** What the viewer fetched for the open file. Binary or oversized files
 * resolve to `none` and fall back to the download affordance. */
type AgentArtifactPreview =
  | { kind: "loading" }
  | { kind: "image"; dataUrl: string }
  | { kind: "text"; text: string }
  | { kind: "none" };

// Files panel width — user-resizable between these bounds (and never past
// roughly half the window), remembered across sessions. The live value is
// the --agent-files-w custom property on .app-shell, which the panel, the
// main card's margin, and the composer all share.
const AGENT_FILES_WIDTH_KEY = "clovy:agent:files-panel-width";
const FILES_PANEL_MIN_W = 300;
const FILES_PANEL_MAX_W = 600;

function clampFilesPanelWidth(width: number) {
  const viewportCap =
    typeof window === "undefined" ? FILES_PANEL_MAX_W : Math.round(window.innerWidth * 0.48);
  const max = Math.max(FILES_PANEL_MIN_W, Math.min(FILES_PANEL_MAX_W, viewportCap));
  return Math.min(Math.max(Math.round(width), FILES_PANEL_MIN_W), max);
}

export function AgentArtifactPanel({
  artifacts,
  state,
  onShowList,
  onOpen,
  onDownload,
  onClose,
}: {
  artifacts: AgentArtifact[];
  state: AgentArtifactPanelState;
  onShowList: () => void;
  onOpen: (artifact: AgentArtifact) => void;
  onDownload: (artifact: AgentArtifact) => void;
  onClose: () => void;
}) {
  const t = useT();
  const artifact = state.view === "file" ? state.artifact : null;
  const [preview, setPreview] = useState<AgentArtifactPreview>({
    kind: "loading",
  });
  const [showSource, setShowSource] = useState(false);
  const [query, setQuery] = useState("");
  const [filterOpen, setFilterOpen] = useState(false);
  const [activeMatchIndex, setActiveMatchIndex] = useState(0);
  const [matchCount, setMatchCount] = useState(0);
  // The slide-in entrance must run once per mount and never again. WebKit
  // replays CSS animations whenever it recreates the renderer (it does this
  // during the sidebar drag's per-frame relayout), which flashed the panel
  // mid-gesture. Once the entrance finishes, data-entered switches the
  // animation off entirely so a renderer rebuild has nothing to replay.
  const [entered, setEntered] = useState(false);
  const panelRef = useRef<HTMLElement>(null);

  // Restore the remembered width once per panel mount. The property lives on
  // .app-shell (not this element) because the main card's slide-over margin
  // and the composer's right inset consume it too.
  useEffect(() => {
    const shell = panelRef.current?.closest(".app-shell");
    if (!(shell instanceof HTMLElement)) return;
    const stored = Number.parseInt(window.localStorage.getItem(AGENT_FILES_WIDTH_KEY) ?? "", 10);
    if (Number.isFinite(stored)) {
      shell.style.setProperty("--agent-files-w", `${clampFilesPanelWidth(stored)}px`);
    }
  }, []);

  // Drag-resize from the panel's left edge, mirroring the sidebar handle:
  // the var tracks the cursor with transitions suppressed (the
  // data-files-resizing attribute), and the final width persists on release.
  const startResize = useCallback((event: ReactPointerEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const shell = event.currentTarget.closest(".app-shell");
    const startWidth = panelRef.current?.offsetWidth;
    if (!(shell instanceof HTMLElement) || !startWidth) return;
    shell.setAttribute("data-files-resizing", "true");
    const startX = event.clientX;
    const onMove = (move: PointerEvent) => {
      const next = clampFilesPanelWidth(startWidth + (startX - move.clientX));
      shell.style.setProperty("--agent-files-w", `${next}px`);
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      shell.removeAttribute("data-files-resizing");
      const finalWidth = panelRef.current?.offsetWidth;
      if (finalWidth) {
        window.localStorage.setItem(AGENT_FILES_WIDTH_KEY, `${finalWidth}`);
      }
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp, { once: true });
  }, []);

  const artifactPath = artifact?.path;
  useEffect(() => {
    setShowSource(false);
    if (!artifactPath) return;
    let cancelled = false;
    setPreview({ kind: "loading" });
    const load: Promise<AgentArtifactPreview> = isPreviewableImagePath(artifactPath)
      ? agentFilePreview(artifactPath).then((dataUrl) =>
          dataUrl ? ({ kind: "image", dataUrl } as const) : ({ kind: "none" } as const),
        )
      : agentFileText(artifactPath).then((text) =>
          text !== null ? ({ kind: "text", text } as const) : ({ kind: "none" } as const),
        );
    void load
      .then((next) => {
        if (!cancelled) setPreview(next);
      })
      .catch(() => {
        if (!cancelled) setPreview({ kind: "none" });
      });
    return () => {
      cancelled = true;
    };
  }, [artifactPath]);

  useEffect(() => {
    setQuery("");
    setFilterOpen(false);
  }, [artifactPath, state.view]);

  const markdown = artifact !== null && isMarkdownPath(artifact.path) && preview.kind === "text";

  // In the list the magnifier filters file names; on a text preview it finds
  // within the document. Images and binaries have nothing to search.
  const searchable = !artifact || preview.kind === "text";
  const filterLabel = artifact ? t("chat.files.findInFile") : t("chat.files.filterFiles");

  // Find-in-file re-renders the whole document, so the highlight trails the
  // keystrokes slightly instead of re-parsing a near-2 MB file on each one.
  // Clearing syncs immediately — Esc/X should unhighlight without lag. The
  // list filter stays live; it only re-renders its rows.
  const [debouncedQuery, setDebouncedQuery] = useState("");
  useEffect(() => {
    if (!query) {
      setDebouncedQuery("");
      return;
    }
    const id = window.setTimeout(() => setDebouncedQuery(query), 150);
    return () => window.clearTimeout(id);
  }, [query]);
  const docHighlight = artifact ? debouncedQuery.trim() || undefined : undefined;

  // Position-aware scroll fades on the document body (same recipe as the
  // dictation history dialog): the header has no divider, so the top fade is
  // what tells you content has scrolled up behind it.
  const bodyRef = useRef<HTMLDivElement>(null);
  const fade = useScrollFade(bodyRef);

  // Count the marks that the active view actually rendered. Markdown syntax
  // can hide source-only text (for example, a link destination), so counting
  // the raw file would make the ordinal disagree with the navigable matches in
  // Preview. A changed query, artifact, or Preview/Source mode starts again at
  // the first visible match.
  useLayoutEffect(() => {
    const matches = docHighlight
      ? bodyRef.current?.querySelectorAll<HTMLElement>("mark[data-search-match-index]")
      : undefined;
    setMatchCount(matches?.length ?? 0);
    setActiveMatchIndex(0);
  }, [artifactPath, debouncedQuery, docHighlight, preview, showSource]);

  useEffect(() => {
    if (matchCount === 0) return;
    const activeMatch = bodyRef.current?.querySelector<HTMLElement>(
      `mark[data-search-match-index="${activeMatchIndex}"]`,
    );
    activeMatch?.scrollIntoView?.({ block: "center", inline: "nearest" });
  }, [activeMatchIndex, matchCount]);

  const navigateMatches = useCallback(
    (direction: -1 | 1) => {
      if (matchCount === 0) return;
      setActiveMatchIndex((current) => (current + direction + matchCount) % matchCount);
    },
    [matchCount],
  );
  // Re-measure when the panel swaps between the artifact preview and the list,
  // or when the preview content changes (the hook re-wires its observers on the
  // element swap; this catches same-element content changes).
  useEffect(() => {
    fade.update();
  }, [fade.update, preview, state.view]);

  const q = query.trim().toLowerCase();
  const visibleArtifacts = q
    ? artifacts.filter((item) => item.name.toLowerCase().includes(q))
    : artifacts;

  return (
    <>
      <div
        className="agent-files-resize-handle"
        role="separator"
        aria-orientation="vertical"
        aria-label={t("chat.files.resizePanel")}
        onPointerDown={startResize}
      />
      <aside
        ref={panelRef}
        className="agent-artifact-panel"
        aria-label={t("chat.files.title")}
        data-entered={entered ? "true" : undefined}
        onAnimationEnd={(event) => {
          if (event.animationName === "agent-artifact-panel-in") setEntered(true);
        }}
      >
        <header className="agent-artifact-panel-bar">
          {artifact ? (
            <button
              type="button"
              className="icon-button"
              aria-label={t("chat.files.allFiles")}
              title={t("chat.files.allFiles")}
              onClick={onShowList}
            >
              <IconChevronLeftSmall size={16} />
            </button>
          ) : null}
          {searchable && filterOpen ? (
            <label className="folders-search agent-artifact-filter">
              <IconMagnifyingGlass size={14} />
              <input
                type="search"
                value={query}
                placeholder={filterLabel}
                aria-label={filterLabel}
                autoFocus
                onChange={(event) => {
                  setQuery(event.currentTarget.value);
                  setMatchCount(0);
                  setActiveMatchIndex(0);
                }}
                onBlur={() => {
                  if (!query.trim()) setFilterOpen(false);
                }}
                onKeyDown={(event) => {
                  if (artifact && event.key === "Enter" && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    event.stopPropagation();
                    navigateMatches(event.shiftKey ? -1 : 1);
                    return;
                  }
                  if (event.key !== "Escape") return;
                  // Esc walks back one step at a time — clear the query,
                  // collapse the filter — before a final Esc (bubbling to
                  // the workspace listener) closes the panel.
                  event.stopPropagation();
                  if (query) setQuery("");
                  else setFilterOpen(false);
                }}
              />
              {artifact && query.trim() ? (
                <span className="agent-artifact-match-navigation">
                  <output className="agent-artifact-match-status" aria-live="polite">
                    {t("chat.files.matchStatus", {
                      current: matchCount > 0 ? activeMatchIndex + 1 : 0,
                      total: matchCount,
                    })}
                  </output>
                  <button
                    type="button"
                    className="icon-button agent-artifact-match-button"
                    aria-label={t("chat.files.previousMatch")}
                    disabled={matchCount === 0}
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => navigateMatches(-1)}
                  >
                    <IconArrowUp size={12} />
                  </button>
                  <button
                    type="button"
                    className="icon-button agent-artifact-match-button"
                    aria-label={t("chat.files.nextMatch")}
                    disabled={matchCount === 0}
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => navigateMatches(1)}
                  >
                    <IconArrowDown size={12} />
                  </button>
                </span>
              ) : null}
              <button
                type="button"
                className="agent-artifact-filter-clear"
                aria-label={query ? t("chat.files.clearFilter") : t("chat.files.closeFilter")}
                title={query ? t("chat.files.clear") : t("common.close")}
                // Mirrors the Esc ladder for the mouse: clear the query
                // first, then collapse back to the magnifier. mousedown is
                // suppressed so clearing doesn't blur (and collapse) the
                // field.
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => {
                  if (query) setQuery("");
                  else setFilterOpen(false);
                }}
              >
                <IconCrossSmall size={12} />
              </button>
            </label>
          ) : (
            <h2 className="agent-artifact-panel-title">
              {artifact ? artifact.name : t("chat.files.title")}
            </h2>
          )}
          {searchable && !filterOpen ? (
            <button
              type="button"
              className="icon-button"
              aria-label={filterLabel}
              title={filterLabel}
              onClick={() => setFilterOpen(true)}
            >
              <IconMagnifyingGlass size={15} />
            </button>
          ) : null}
          {artifact ? (
            <button
              type="button"
              className="icon-button"
              aria-label={t("chat.files.downloadNamed", { name: artifact.name })}
              title={t("chat.media.download")}
              onClick={() => onDownload(artifact)}
            >
              <IconArrowInbox size={15} />
            </button>
          ) : null}
          <button
            type="button"
            className="icon-button"
            aria-label={t("chat.files.closeFiles")}
            title={t("common.close")}
            onClick={onClose}
          >
            <IconCrossMedium size={15} />
          </button>
        </header>
        {markdown ? (
          <div className="agent-artifact-panel-mode">
            <SegmentedControl
              aria-label={t("chat.files.fileView")}
              value={showSource ? "source" : "preview"}
              onValueChange={(value) => setShowSource(value === "source")}
              options={[
                { value: "preview", label: t("chat.files.preview") },
                { value: "source", label: t("chat.files.source") },
              ]}
            />
          </div>
        ) : null}
        {artifact ? (
          <div
            ref={bodyRef}
            className="agent-artifact-panel-body scroll-fade-mask"
            data-kind={preview.kind}
            {...fade.props}
          >
            {preview.kind === "loading" ? (
              <Spinner />
            ) : preview.kind === "image" ? (
              <img
                className="agent-artifact-panel-image"
                src={preview.dataUrl}
                alt={artifact.name}
              />
            ) : preview.kind === "text" && markdown && !showSource ? (
              <MarkdownContent
                markdown={preview.text}
                highlight={docHighlight}
                activeHighlightIndex={activeMatchIndex}
              />
            ) : preview.kind === "text" ? (
              <pre className="agent-artifact-source">
                {docHighlight
                  ? highlightText(preview.text, docHighlight, "source", {
                      activeIndex: activeMatchIndex,
                      nextIndex: 0,
                    } satisfies HighlightCursor)
                  : preview.text}
              </pre>
            ) : (
              <div className="agent-artifact-panel-empty">
                <p>{t("chat.files.noPreview")}</p>
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => onDownload(artifact)}
                >
                  <IconArrowInbox size={14} />
                  {t("chat.media.download")}
                </button>
              </div>
            )}
          </div>
        ) : (
          <>
            <div
              ref={bodyRef}
              className="agent-artifact-panel-body scroll-fade-mask"
              data-kind="list"
              {...fade.props}
            >
              {visibleArtifacts.length ? (
                <ul className="agent-artifact-panel-list">
                  {visibleArtifacts.map((item) => {
                    const ArtifactIcon = fileTypeIconComponent(item.path);
                    return (
                      <li key={item.path}>
                        <button
                          type="button"
                          className="agent-artifact-row"
                          onClick={() => onOpen(item)}
                        >
                          <span className="agent-artifact-icon">
                            <ArtifactIcon size={18} />
                          </span>
                          <span className="agent-artifact-row-name">{item.name}</span>
                          <span className="agent-artifact-row-meta">{formatBytes(item.size)}</span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              ) : (
                <p className="agent-artifact-search-empty">{t("chat.files.noMatches")}</p>
              )}
            </div>
          </>
        )}
      </aside>
    </>
  );
}

function isPreviewableImagePath(path: string) {
  return /\.(png|jpe?g|gif|webp)$/i.test(path);
}

function isMarkdownPath(path: string) {
  return /\.(md|markdown|mdx)$/i.test(path);
}
