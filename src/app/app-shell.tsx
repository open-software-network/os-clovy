import { IconCalendarClock } from "central-icons/IconCalendarClock";
import { IconMicrophone } from "central-icons/IconMicrophone";
import { IconNoteText } from "central-icons/IconNoteText";
import { IconProjects } from "central-icons/IconProjects";
import { IconSettingsGear4 } from "central-icons/IconSettingsGear4";
import { IconZap } from "central-icons/IconZap";
import type { ReactNode } from "react";
import { t } from "../i18n";
import type { FolderDto, NoteDto, NoteListItemDto } from "../lib/tauri";
import { normalizeTabNav } from "./tabs/tabs";
export const SIDEBAR_DEFAULT_WIDTH = 240;
export const SIDEBAR_MIN_WIDTH = 188;
export const SIDEBAR_MAX_WIDTH = 320;
export const SIDEBAR_COLLAPSE_WIDTH = 160;
export const ACCESSIBILITY_PERMISSION_REFRESH_INTERVAL_MS = 1000;
export const SYSTEM_AUDIO_PERMISSION_REFRESH_INTERVAL_MS = 1000;
export const SYSTEM_AUDIO_PERMISSION_REFRESH_TIMEOUT_MS = 120_000;
export const MEETING_START_LISTENER_RETRY_DELAYS_MS = [250, 1_000, 5_000] as const;
// Start-expiry copy is read at call time so it follows the interface language.
export const meetingStartRequestExpiredMessage = () => t("app.recording.startExpired");
// Floor for the note card so the sidebar can't be dragged wide enough to
// crush it into a sliver — it always keeps a usable width plus its gutters.
export const MAIN_PANEL_MIN_WIDTH = 420;

export function noteHasDownloadableAudio(note: NoteDto): boolean {
  const audioSources = note.audioSources?.length
    ? note.audioSources
    : note.audio
      ? [note.audio]
      : [];
  return audioSources.some((audio) => audio.format === "wav" && audio.sizeBytes > 0);
}

// Largest the sidebar may grow given the live window width: never past its own
// cap, and never so far that the main panel drops below its floor. Falls back
// to the sidebar min on very narrow windows where both can't be satisfied.
export function sidebarMaxWidth() {
  return Math.max(
    SIDEBAR_MIN_WIDTH,
    Math.min(SIDEBAR_MAX_WIDTH, window.innerWidth - MAIN_PANEL_MIN_WIDTH),
  );
}

export const TAB_ICON_SIZE = 14;

export type RecordingInactivityPrompt = {
  sessionId: string;
  expiresAt: number;
};

export type AgentRecorderRequestPayload = {
  requestId?: unknown;
  action?: unknown;
  sourceMode?: unknown;
  profile?: unknown;
  noteId?: unknown;
  sessionId?: unknown;
  recoveryAction?: unknown;
};

export function tabMeta(
  snapshot: Parameters<typeof normalizeTabNav>[0],
  notes: NoteListItemDto[],
  folders: FolderDto[],
  settingsSectionLabel?: string,
): { title: string; icon: ReactNode } {
  const nav = normalizeTabNav(snapshot);
  switch (nav.view) {
    case "meetings": {
      const note = nav.noteId ? notes.find((n) => n.id === nav.noteId) : undefined;
      return {
        title: note?.title?.trim() || t("app.newNote"),
        icon: <IconNoteText size={TAB_ICON_SIZE} />,
      };
    }
    case "folders": {
      const folder = nav.folderId ? folders.find((f) => f.id === nav.folderId) : undefined;
      return {
        title: folder?.name?.trim() || t("app.nav.projects"),
        icon: <IconProjects size={TAB_ICON_SIZE} />,
      };
    }
    case "all-notes":
      return {
        title: t("app.nav.allNotes"),
        icon: <IconNoteText size={TAB_ICON_SIZE} />,
      };
    case "routines":
      return {
        title: t("app.nav.routines"),
        icon: <IconZap size={TAB_ICON_SIZE} />,
      };
    case "dictation":
      return {
        title: t("app.nav.dictation"),
        icon: <IconMicrophone size={TAB_ICON_SIZE} />,
      };
    case "today":
      return {
        title: t("app.nav.today"),
        icon: <IconCalendarClock size={TAB_ICON_SIZE} />,
      };
    case "settings":
      return {
        // Surface the active settings section (e.g. "MCP servers") in the tab
        // strip so the label says what you are looking at, not just "Settings".
        title: settingsSectionLabel?.trim() || t("common.settings"),
        icon: <IconSettingsGear4 size={TAB_ICON_SIZE} />,
      };
    default:
      return {
        title: t("app.nav.notes"),
        icon: <IconNoteText size={TAB_ICON_SIZE} />,
      };
  }
}
