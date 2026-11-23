import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconRobot2 } from "central-icons/IconRobot2";
import { IconChevronLeftSmall } from "central-icons/IconChevronLeftSmall";
import { IconAudio } from "central-icons/IconAudio";
import { IconBrain2 } from "central-icons/IconBrain2";
import { IconBrainSideview } from "central-icons/IconBrainSideview";
import { IconCircleInfo } from "central-icons/IconCircleInfo";
import { IconColorPalette } from "central-icons/IconColorPalette";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { IconMicrophone } from "central-icons/IconMicrophone";
import { IconMicrophoneSparkle } from "central-icons/IconMicrophoneSparkle";
import { IconNoteText } from "central-icons/IconNoteText";
import { IconPlugin1 } from "central-icons/IconPlugin1";
import { IconProjects } from "central-icons/IconProjects";
import { IconSettingsGear4 } from "central-icons/IconSettingsGear4";
import { IconShortcut } from "central-icons/IconShortcut";
import { IconLiveActivity } from "central-icons/IconLiveActivity";
import { IconZap } from "central-icons/IconZap";
import { IconCalendarClock } from "central-icons/IconCalendarClock";
import { activityStatus } from "../../lib/activity-capture";
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type RefObject,
  type ReactNode,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { ClovyWordmark } from "../brand/ClovyLogo";
import { attachScrollThumbFade } from "../../lib/scroll-thumb-fade";
import { useScrollFade } from "../../lib/use-scroll-fade";
import { useRecordingPresenceBounds } from "../../lib/recording-presence-bounds";
import { isPrimaryShortcut, primaryShortcutLabel } from "../../lib/platform";
import type { NoteListItemDto, RecordingStatusDto } from "../../lib/tauri";
import { isMacLikePlatform, isWindowsPlatform } from "../../lib/platform";
import { settingsTabsForPlatform, type SettingsTab } from "../settings/settings-config";
import { combineSourceAudioLevels, Waveform } from "../recorder/Waveform";
import { type MessageKey, useT } from "../../i18n";

export type SidebarView =
  | "notes"
  | "meetings"
  | "all-notes"
  | "settings"
  | "folders"
  | "dictation"
  | "routines"
  | "today";

type SidebarProps = {
  notes: NoteListItemDto[];
  activeView: SidebarView;
  // Settings is its own page reached from the footer menu; these default so
  // tests that mount the sidebar for non-settings views can skip the plumbing.
  settingsTab?: SettingsTab;
  onSettingsTabChange?: (tab: SettingsTab) => void;
  onChangeView: (view: SidebarView) => void;
  // Returns to wherever the user was before opening settings (falls back to
  // Notes when not wired, e.g. unit tests).
  onExitSettings?: () => void;
  onSelectNote: (noteId: string) => void;
  recordingStatus?: RecordingStatusDto | null;
  recordingTitle?: string;
  onOpenRecording?: () => void;
  collapsed?: boolean;
  footerAccessory?: ReactNode;
};

type CommandPromptItem = {
  id: string;
  label: string;
  meta?: string;
  icon: ReactNode;
  searchText: string;
  action: () => void;
};

type CommandPromptGroup = {
  title: string;
  items: CommandPromptItem[];
};

const SETTINGS_SIDEBAR_GROUPS: {
  id: string;
  title: MessageKey;
  items: { id: SettingsTab; label: MessageKey; icon: ReactNode }[];
}[] = [
  {
    id: "personal",
    title: "shell.settingsNav.group.personal",
    items: [
      {
        id: "general",
        label: "shell.settingsNav.general",
        icon: <IconSettingsGear4 size={16} />,
      },
      {
        id: "appearance",
        label: "shell.settingsNav.appearance",
        icon: <IconColorPalette size={16} />,
      },
      {
        id: "shortcuts",
        label: "shell.settingsNav.shortcuts",
        icon: <IconShortcut size={16} />,
      },
      {
        id: "activity",
        label: "activity.tab.label",
        icon: <IconLiveActivity size={16} />,
      },
    ],
  },
  {
    id: "audio",
    title: "shell.settingsNav.group.audio",
    items: [
      {
        id: "dictation",
        label: "shell.settingsNav.dictation",
        icon: <IconMicrophoneSparkle size={16} />,
      },
      { id: "audio", label: "shell.settingsNav.audio", icon: <IconAudio size={16} /> },
    ],
  },
  {
    id: "ai",
    title: "shell.settingsNav.group.ai",
    items: [
      { id: "models", label: "shell.settingsNav.models", icon: <IconBrain2 size={16} /> },
      { id: "agent", label: "shell.settingsNav.agent", icon: <IconRobot2 size={16} /> },
      {
        id: "memory",
        label: "shell.settingsNav.memory",
        icon: <IconBrainSideview size={16} />,
      },
      {
        id: "mcp",
        label: "mcpServer.tab",
        icon: <IconPlugin1 size={16} />,
      },
      {
        id: "connectors",
        label: "shell.settingsNav.plugins",
        icon: <IconPlugin1 size={16} />,
      },
    ],
  },
  {
    id: "app",
    title: "shell.settingsNav.group.app",
    items: [{ id: "about", label: "shell.settingsNav.about", icon: <IconCircleInfo size={16} /> }],
  },
];

/**
 * Settings tabs introduced by the admin-surfaces PR, hidden from the nav until
 * they're stabilized. The pre-PR tabs (General, Shortcuts, Dictation,
 * Audio, Models, Agent, Installed skills, About) stay visible, plus External
 * skill directories (PR-new but verified working). These are hidden, not
 * removed: tabs, panels, and logic are all intact. Re-enable one by deleting its
 * id here; restore the full nav by deleting this set and the `.filter` in
 * SettingsSidebar that uses it.
 */
export const HIDDEN_SETTINGS_TABS: ReadonlySet<SettingsTab> = new Set();

export function Sidebar({
  notes,
  activeView,
  settingsTab = "general",
  onSettingsTabChange,
  onChangeView,
  onExitSettings,
  onSelectNote,
  recordingStatus,
  recordingTitle,
  onOpenRecording,
  collapsed = false,
  footerAccessory,
}: SidebarProps) {
  const t = useT();
  const sidebarSearchRef = useRef<HTMLInputElement>(null);
  const commandInputRef = useRef<HTMLInputElement>(null);
  const [commandPromptOpen, setCommandPromptOpen] = useState(false);
  const [commandQuery, setCommandQuery] = useState("");
  const [commandActiveIndex, setCommandActiveIndex] = useState(0);
  const [activitySupported, setActivitySupported] = useState(
    () => isMacLikePlatform() || isWindowsPlatform(),
  );

  useEffect(() => {
    void activityStatus()
      .then((status) => setActivitySupported(status.supported))
      .catch(() => {});
  }, []);
  const searchShortcut = primaryShortcutLabel("K");
  const inSettings = activeView === "settings";
  const commandPromptGroups = useMemo<CommandPromptGroup[]>(() => {
    const normalized = normalizeCommandQuery(commandQuery);
    const matches = (item: CommandPromptItem) =>
      !normalized || item.searchText.includes(normalized);

    const recentItems: CommandPromptItem[] = [
      ...notes.slice(0, 5).map((note) => {
        const title = note.title.trim() || t("shell.sidebar.newNote");
        return {
          id: `note:${note.id}`,
          label: title,
          meta: t("shell.command.meetingNote"),
          icon: <IconNoteText size={15} />,
          searchText: normalizeCommandQuery(`${title} ${note.preview}`),
          action: () => onSelectNote(note.id),
        };
      }),
    ]
      .filter(matches)
      .slice(0, 6);

    const quickItems: CommandPromptItem[] = [
      // "Open recording" only makes sense while a recording is live, mirroring
      // the sidebar recording indicator that renders under the same condition.
      ...(onOpenRecording && recordingStatus
        ? [
            {
              id: "quick:open-recording",
              label: t("shell.recording.open"),
              icon: <IconAudio size={15} />,
              searchText: normalizeCommandQuery(
                `open recording meeting audio ${t("shell.recording.open")}`,
              ),
              action: onOpenRecording,
            } satisfies CommandPromptItem,
          ]
        : []),
      {
        id: "quick:meetings",
        label: t("shell.command.goMeetingNotes"),
        icon: <IconNoteText size={15} />,
        searchText: normalizeCommandQuery(
          `meeting notes meetings go to ${t("shell.command.goMeetingNotes")}`,
        ),
        action: () => onChangeView("notes"),
      },
      {
        id: "quick:today",
        label: t("shell.command.goToday"),
        icon: <IconCalendarClock size={15} />,
        searchText: normalizeCommandQuery(
          `today hoje activity timeline go to ${t("shell.command.goToday")}`,
        ),
        action: () => onChangeView("today"),
      },
      {
        id: "quick:projects",
        label: t("shell.command.goProjects"),
        icon: <IconProjects size={15} />,
        searchText: normalizeCommandQuery(
          `projects folders go to ${t("shell.command.goProjects")}`,
        ),
        action: () => onChangeView("folders"),
      },
      {
        id: "quick:dictation",
        label: t("shell.command.goDictation"),
        icon: <IconMicrophone size={15} />,
        searchText: normalizeCommandQuery(`dictation go to ${t("shell.command.goDictation")}`),
        action: () => onChangeView("dictation"),
      },
      {
        id: "quick:connectors",
        label: t("shell.command.goConnectors"),
        icon: <IconPlugin1 size={15} />,
        searchText: normalizeCommandQuery(
          `connectors plugins computer use google linear go to ${t("shell.command.goConnectors")}`,
        ),
        action: () => {
          onSettingsTabChange?.("connectors");
          onChangeView("settings");
        },
      },
      {
        id: "quick:settings",
        label: t("shell.command.openSettings"),
        icon: <IconSettingsGear4 size={15} />,
        searchText: normalizeCommandQuery(
          `open settings preferences ${t("shell.command.openSettings")}`,
        ),
        action: () => onChangeView("settings"),
      },
      // Per-tab settings jumps surface only once a query is typed so ten
      // rows don't flood the default Quick actions list. General and
      // Appearance carry their row-level terms so "theme" or "permissions"
      // still finds the right tab.
      ...(normalized
        ? settingsTabsForPlatform(activitySupported)
            .filter((tab) => !HIDDEN_SETTINGS_TABS.has(tab.id))
            .map(
              (tab): CommandPromptItem => ({
                id: `quick:settings-${tab.id}`,
                label: t("shell.command.settingsTab", { label: tab.label }),
                icon: <IconSettingsGear4 size={15} />,
                searchText: normalizeCommandQuery(
                  tab.id === "general"
                    ? `settings general permissions privacy ${tab.label}`
                    : tab.id === "appearance"
                      ? `settings appearance theme accent text size dark light mode ${tab.label}`
                      : tab.id === "activity"
                        ? `settings activity capture permissions ${tab.label}`
                        : `settings ${tab.label}`,
                ),
                action: () => {
                  onSettingsTabChange?.(tab.id);
                  onChangeView("settings");
                },
              }),
            )
        : []),
    ].filter(matches);

    return [
      { title: t("shell.command.group.recents"), items: recentItems },
      { title: t("shell.command.group.quickActions"), items: quickItems },
    ].filter((group) => group.items.length > 0);
  }, [
    activitySupported,
    commandQuery,
    notes,
    onChangeView,
    onOpenRecording,
    onSelectNote,
    onSettingsTabChange,
    recordingStatus,
    t,
  ]);

  const commandPromptItems = commandPromptGroups.flatMap((group) => group.items);

  useEffect(() => {
    setCommandActiveIndex(0);
  }, [commandPromptOpen, commandQuery]);

  useEffect(() => {
    if (commandActiveIndex < commandPromptItems.length) return;
    setCommandActiveIndex(Math.max(commandPromptItems.length - 1, 0));
  }, [commandActiveIndex, commandPromptItems.length]);

  function openCommandPrompt() {
    setCommandQuery("");
    setCommandActiveIndex(0);
    setCommandPromptOpen(true);
  }

  function closeCommandPrompt() {
    setCommandPromptOpen(false);
    sidebarSearchRef.current?.focus();
  }

  function runCommandPromptItem(item: CommandPromptItem) {
    closeCommandPrompt();
    item.action();
  }

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (!isSearchShortcut(event)) return;
      if (document.querySelector('[role="dialog"]')) return;
      event.preventDefault();
      openCommandPrompt();
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  useEffect(() => {
    if (!commandPromptOpen) return;
    const frame = window.requestAnimationFrame(() => {
      commandInputRef.current?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [commandPromptOpen]);

  return (
    <aside
      className="sidebar"
      data-collapsed={collapsed}
      data-mode={inSettings ? "settings" : "default"}
    >
      {inSettings ? null : (
        <header className="sidebar-header">
          <a className="sidebar-brand" href="#" aria-label="Clovy">
            <ClovyWordmark className="sidebar-brand-mark" label="Clovy" variant="mono" />
          </a>
          {recordingStatus ? (
            <SidebarRecordingIndicator
              status={recordingStatus}
              title={recordingTitle ?? t("shell.sidebar.newNote")}
              onOpen={onOpenRecording}
            />
          ) : null}
        </header>
      )}

      {inSettings ? (
        <SettingsSidebarNav
          activeTab={settingsTab}
          activitySupported={activitySupported}
          onSelectTab={(tab) => onSettingsTabChange?.(tab)}
          onBack={() => (onExitSettings ? onExitSettings() : onChangeView("today"))}
        />
      ) : (
        <>
          <label
            className="sidebar-search"
            onMouseDown={(event) => {
              event.preventDefault();
              openCommandPrompt();
            }}
          >
            <IconMagnifyingGlass size={15} />
            <input
              type="search"
              ref={sidebarSearchRef}
              onKeyDown={(event) => {
                if (event.key !== "Enter" && event.key !== " ") return;
                event.preventDefault();
                openCommandPrompt();
              }}
              placeholder={t("common.search")}
              aria-label={t("common.search")}
              readOnly
            />
            <span className="sidebar-search-kbd" aria-hidden="true">
              {searchShortcut}
            </span>
          </label>

          <nav className="sidebar-nav" aria-label={t("shell.sidebar.primaryNav")}>
            <button
              type="button"
              className="sidebar-nav-item"
              data-active={activeView === "today"}
              aria-current={activeView === "today" ? "page" : undefined}
              onClick={() => onChangeView("today")}
            >
              <span className="sidebar-nav-icon">
                <IconCalendarClock size={15} />
              </span>
              <span className="sidebar-nav-label">{t("shell.sidebar.today")}</span>
            </button>
            <button
              type="button"
              className="sidebar-nav-item"
              data-active={activeView === "meetings" || activeView === "notes"}
              aria-current={
                activeView === "meetings" || activeView === "notes" ? "page" : undefined
              }
              onClick={() => {
                onChangeView("notes");
              }}
            >
              <span className="sidebar-nav-icon">
                <IconNoteText size={15} />
              </span>
              <span className="sidebar-nav-label">{t("shell.sidebar.meetingNotes")}</span>
            </button>
            <button
              type="button"
              className="sidebar-nav-item"
              data-active={activeView === "folders"}
              aria-current={activeView === "folders" ? "page" : undefined}
              onClick={() => onChangeView("folders")}
            >
              <span className="sidebar-nav-icon">
                <IconProjects size={15} />
              </span>
              <span className="sidebar-nav-label">{t("shell.sidebar.projects")}</span>
            </button>
            <button
              type="button"
              className="sidebar-nav-item"
              data-active={activeView === "dictation"}
              aria-current={activeView === "dictation" ? "page" : undefined}
              onClick={() => onChangeView("dictation")}
            >
              <span className="sidebar-nav-icon">
                <IconMicrophone size={16} />
              </span>
              <span className="sidebar-nav-label">{t("shell.sidebar.dictation")}</span>
            </button>
            <button
              type="button"
              className="sidebar-nav-item"
              data-active={activeView === "routines"}
              aria-current={activeView === "routines" ? "page" : undefined}
              onClick={() => onChangeView("routines")}
            >
              <span className="sidebar-nav-icon">
                <IconZap size={16} />
              </span>
              <span className="sidebar-nav-label">{t("shell.sidebar.routines")}</span>
            </button>
          </nav>
        </>
      )}

      <footer className="sidebar-footer">
        {footerAccessory}
        <SidebarSettingsButton onOpenSettings={() => onChangeView("settings")} />
      </footer>

      {commandPromptOpen ? (
        <CommandPrompt
          inputRef={commandInputRef}
          query={commandQuery}
          groups={commandPromptGroups}
          items={commandPromptItems}
          activeIndex={commandActiveIndex}
          onQueryChange={setCommandQuery}
          onActiveIndexChange={setCommandActiveIndex}
          onClose={closeCommandPrompt}
          onSelect={runCommandPromptItem}
        />
      ) : null}
    </aside>
  );
}

function SidebarRecordingIndicator({
  status,
  title,
  onOpen,
}: {
  status: RecordingStatusDto;
  title: string;
  onOpen?: () => void;
}) {
  const t = useT();
  const buttonRef = useRef<HTMLButtonElement>(null);
  const recording = status.state === "recording";
  useRecordingPresenceBounds(buttonRef);
  const meterLevel =
    status.sources && status.sources.length > 0
      ? combineSourceAudioLevels(status.sources)
      : status.level;

  return (
    <button
      ref={buttonRef}
      type="button"
      className="sidebar-recording-indicator"
      data-state={status.state}
      onClick={onOpen}
      aria-label={t("shell.recording.openNamed", { title })}
      title={t("shell.recording.open")}
    >
      <span className="sidebar-recording-dot" aria-hidden />
      <Waveform level={meterLevel} sessionId={status.sessionId} active={recording} />
    </button>
  );
}

function SettingsSidebarNav({
  activeTab,
  activitySupported,
  onSelectTab,
  onBack,
}: {
  activeTab: SettingsTab;
  activitySupported: boolean;
  onSelectTab: (tab: SettingsTab) => void;
  onBack: () => void;
}) {
  const t = useT();
  // Hide the admin-surfaces-PR tabs until stabilized. Empty groups drop out so
  // their headers don't render.
  const supportedTabs = new Set(settingsTabsForPlatform(activitySupported).map((tab) => tab.id));
  const groups = SETTINGS_SIDEBAR_GROUPS.map((group) => ({
    ...group,
    items: group.items.filter(
      (item) => !HIDDEN_SETTINGS_TABS.has(item.id) && supportedTabs.has(item.id),
    ),
  })).filter((group) => group.items.length > 0);

  return (
    <section className="sidebar-section sidebar-settings-section" aria-label={t("common.settings")}>
      <button type="button" className="sidebar-nav-item sidebar-settings-back" onClick={onBack}>
        <span className="sidebar-nav-icon">
          <IconChevronLeftSmall size={15} />
        </span>
        <span className="sidebar-nav-label">{t("shell.settingsNav.back")}</span>
      </button>
      {groups.map((group) => (
        <div key={group.id} className="sidebar-settings-group">
          <div className="section-title">
            <span className="section-title-label">{t(group.title)}</span>
          </div>
          <nav
            className="sidebar-nav"
            aria-label={t("shell.settingsNav.groupLabel", { group: t(group.title) })}
          >
            {group.items.map((tab) => (
              <button
                key={tab.id}
                type="button"
                className="sidebar-nav-item"
                data-active={activeTab === tab.id}
                aria-current={activeTab === tab.id ? "page" : undefined}
                onClick={() => onSelectTab(tab.id)}
              >
                <span className="sidebar-nav-icon" aria-hidden>
                  {tab.icon}
                </span>
                <span className="sidebar-nav-label">{t(tab.label)}</span>
              </button>
            ))}
          </nav>
        </div>
      ))}
    </section>
  );
}

function CommandPrompt({
  inputRef,
  query,
  groups,
  items,
  activeIndex,
  onQueryChange,
  onActiveIndexChange,
  onClose,
  onSelect,
}: {
  inputRef: RefObject<HTMLInputElement>;
  query: string;
  groups: CommandPromptGroup[];
  items: CommandPromptItem[];
  activeIndex: number;
  onQueryChange: (value: string) => void;
  onActiveIndexChange: (index: number) => void;
  onClose: () => void;
  onSelect: (item: CommandPromptItem) => void;
}) {
  const t = useT();
  const resultsRef = useRef<HTMLDivElement>(null);
  const fade = useScrollFade(resultsRef);
  // Native-overlay scrollbar feel, same as the main content areas: the custom
  // webkit thumb fades in on scroll/hover and back out when idle (see
  // scroll-thumb-fade.ts and the --thumb-alpha rules in app.css).
  useEffect(() => {
    const el = resultsRef.current;
    if (!el) return;
    return attachScrollThumbFade(el);
  }, []);
  // Re-measure the edge fades when the query or result groups change.
  useEffect(() => {
    fade.update();
  }, [fade.update, query, groups]);

  // Escape always closes, from anywhere — the prompt has no focus trap, so a
  // handler bound to the input alone would miss Esc once focus moves to a row,
  // the clear button, or out to the background. A window listener (mounted only
  // while the prompt is open) guarantees there's no way to get stuck in here.
  // It runs in the capture phase and calls stopPropagation so the prompt claims
  // Escape before an earlier-registered window handler also reacts to it.
  useEffect(() => {
    function onWindowKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }
    }
    window.addEventListener("keydown", onWindowKeyDown, true);
    return () => window.removeEventListener("keydown", onWindowKeyDown, true);
  }, [onClose]);

  // Scroll the active row into view, but only from keyboard navigation — doing
  // it on mouse-hover activation would fight the pointer and cause jumps.
  function moveActive(nextIndex: number) {
    onActiveIndexChange(nextIndex);
    window.requestAnimationFrame(() => {
      document
        .getElementById(`command-prompt-item-${nextIndex}`)
        ?.scrollIntoView({ block: "nearest" });
    });
  }

  function onKeyDown(event: ReactKeyboardEvent<HTMLInputElement>) {
    // Escape is handled by a window listener (see above) so it works no matter
    // where focus is; here we only drive list navigation from the input.
    if (event.key === "ArrowDown") {
      event.preventDefault();
      if (items.length === 0) return;
      moveActive(Math.min(activeIndex + 1, items.length - 1));
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      if (items.length === 0) return;
      moveActive(Math.max(activeIndex - 1, 0));
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      const item = items[activeIndex];
      if (item) onSelect(item);
    }
  }

  let itemIndex = 0;

  // Portal to the document body so the prompt renders above the whole app,
  // never trapped inside the sidebar's DOM subtree. The collapsed sidebar is
  // `display: none` (see .app-shell[data-sidebar="collapsed"] .sidebar), which
  // would otherwise hide this overlay along with it — so ⌘K must open a prompt
  // that lives outside the sidebar to fire regardless of the sidebar state.
  return createPortal(
    <div
      className="command-prompt-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="command-prompt"
        role="dialog"
        aria-modal="true"
        aria-label={t("common.search")}
      >
        <label className="command-prompt-search">
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(event) => onQueryChange(event.currentTarget.value)}
            onKeyDown={onKeyDown}
            placeholder={t("shell.command.placeholder")}
            aria-label={t("common.search")}
            aria-activedescendant={
              items[activeIndex] ? `command-prompt-item-${activeIndex}` : undefined
            }
          />
          {query ? (
            <button
              type="button"
              className="command-prompt-clear"
              aria-label={t("shell.command.clearSearch")}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => {
                onQueryChange("");
                inputRef.current?.focus();
              }}
            >
              <IconCrossSmall size={13} />
            </button>
          ) : null}
        </label>
        <div className="command-prompt-results-wrap scroll-fade" {...fade.props}>
          <div className="command-prompt-results" ref={resultsRef}>
            {groups.length > 0 ? (
              groups.map((group) => (
                <section className="command-prompt-group" key={group.title}>
                  <div className="command-prompt-group-title">{group.title}</div>
                  <div className="command-prompt-group-list">
                    {group.items.map((item) => {
                      const index = itemIndex;
                      itemIndex += 1;
                      return (
                        <button
                          type="button"
                          id={`command-prompt-item-${index}`}
                          className="command-prompt-item"
                          data-active={index === activeIndex}
                          key={item.id}
                          onMouseEnter={() => onActiveIndexChange(index)}
                          onFocus={() => onActiveIndexChange(index)}
                          onClick={() => onSelect(item)}
                        >
                          <span className="command-prompt-item-icon">{item.icon}</span>
                          <span className="command-prompt-item-label">{item.label}</span>
                          {item.meta ? (
                            <span className="command-prompt-item-meta">{item.meta}</span>
                          ) : null}
                        </button>
                      );
                    })}
                  </div>
                </section>
              ))
            ) : (
              <div className="command-prompt-empty">
                {t("shell.command.noResults", { query: query.trim() })}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}

function normalizeCommandQuery(value: string) {
  return value.trim().toLowerCase();
}

function isSearchShortcut(event: KeyboardEvent) {
  return event.key.toLowerCase() === "k" && isPrimaryShortcut(event);
}

// The footer entry point: one row that opens the settings page.
function SidebarSettingsButton({ onOpenSettings }: { onOpenSettings: () => void }) {
  const t = useT();
  return (
    <div className="sidebar-identity-wrap">
      <button type="button" className="sidebar-nav-item sidebar-identity" onClick={onOpenSettings}>
        <IconSettingsGear4 size={16} className="sidebar-nav-icon" />
        <span className="sidebar-nav-label">{t("common.settings")}</span>
      </button>
    </div>
  );
}
