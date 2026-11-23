import { getCurrentWindow } from "@tauri-apps/api/window";
import type { PointerEvent as ReactPointerEvent } from "react";
import { useT } from "../i18n";
import { isPrimaryShortcut } from "../lib/platform";
import type {
  BootstrapResponse,
  NoteDto,
  RecordingSourceMode,
  RecordingSourceReadinessDto,
  RecordingStatusDto,
} from "../lib/tauri";
export function SidebarToggleGlyph() {
  const t = useT();
  return (
    <svg
      className="sidebar-toggle-glyph"
      width={18}
      height={18}
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden
    >
      <title>{t("app.sidebar.toggle")}</title>
      <path
        d="M3 8C3 6.34315 4.34315 5 6 5H18C19.6569 5 21 6.34315 21 8V16C21 17.6569 19.6569 19 18 19H6C4.34315 19 3 17.6569 3 16V8Z"
        stroke="currentColor"
        strokeWidth={2}
        strokeLinejoin="round"
      />
      <line
        className="sidebar-toggle-divider"
        x1={9}
        y1={5}
        x2={9}
        y2={19}
        stroke="currentColor"
        strokeWidth={2}
        strokeLinecap="round"
      />
    </svg>
  );
}

export function handleTitlebarPointerDown(event: ReactPointerEvent<HTMLDivElement>) {
  if (event.button !== 0 || event.detail > 1) return;
  event.preventDefault();
  void getCurrentWindow()
    .startDragging()
    // biome-ignore lint/suspicious/noConsole: surfacing a drag failure is a deliberate diagnostic
    .catch((error: unknown) => console.warn("Failed to start window drag", error));
}

export function revealMainWindowForMeetingStartError() {
  const main = getCurrentWindow();
  void Promise.allSettled([main.show(), main.unminimize(), main.setFocus()]);
}

export function isDeniedPermission(state?: string) {
  return state === "denied" || state === "restricted";
}

// TCC grants are bundle-scoped. The readiness probe reads the main app's
// authorization, which is the only grant relevant to note recording. Once
// that probe has reported, never let the dictation helper's separate grant
// override it. The helper remains a launch-time fallback before readiness is
// available. `not_determined` stays startable so the main app can prompt.
export function isMicrophoneRecordingBlocked(
  helperStatus: string | undefined,
  readiness: RecordingSourceReadinessDto | undefined,
) {
  const readinessState = readiness?.sources.find(
    (source) => source.source === "microphone",
  )?.permissionState;
  return readinessState === undefined
    ? isDeniedPermission(helperStatus)
    : isDeniedPermission(readinessState);
}

// Accessibility is a plain bool from the helper (AXIsProcessTrusted),
// surfaced as "granted" | "missing" — not the mic's denied/restricted
// vocabulary. Treat any known non-granted value as blocked so the paste
// permission banner actually shows when access is missing. Undefined stays
// non-blocking so the banner doesn't flash before the helper's first report.
export function isAccessibilityBlocked(state?: string) {
  return state !== undefined && state !== "granted";
}

export function isCreateNoteShortcut(event: KeyboardEvent) {
  // Primary modifier + Shift + N. isPrimaryShortcut rejects Shift, so check
  // the primary modifier with Shift masked off, then require Shift on top.
  return (
    event.key.toLowerCase() === "n" &&
    event.shiftKey &&
    isPrimaryShortcut({
      metaKey: event.metaKey,
      ctrlKey: event.ctrlKey,
      altKey: event.altKey,
      shiftKey: false,
    })
  );
}

export function stringPayloadValue(value: unknown) {
  return typeof value === "string" ? value : undefined;
}

export function recordingToStatus(recording: {
  id: string;
  noteId?: string;
  sourceMode?: RecordingStatusDto["sourceMode"];
  state: RecordingStatusDto["state"];
  elapsedMs: number;
  level: RecordingStatusDto["level"];
  livePreviewEnabled?: RecordingStatusDto["livePreviewEnabled"];
  sources?: RecordingStatusDto["sources"];
  warnings?: RecordingStatusDto["warnings"];
}): RecordingStatusDto {
  return {
    sessionId: recording.id,
    noteId: recording.noteId,
    sourceMode: recording.sourceMode,
    state: recording.state,
    elapsedMs: recording.elapsedMs,
    level: recording.level,
    silenceWarning: false,
    bytesWritten: 0,
    livePreviewEnabled: recording.livePreviewEnabled ?? false,
    sources: recording.sources,
    warnings: recording.warnings,
  };
}

export function startingRecordingStatus(
  noteId: string,
  sourceMode: RecordingSourceMode,
): RecordingStatusDto {
  const sources: RecordingStatusDto["sources"] = [
    {
      source: "microphone",
      state: "starting",
      elapsedMs: 0,
      bytesWritten: 0,
      level: { peak: 0, rms: 0, recentPeaks: [] },
      silenceWarning: false,
      pathFinalized: false,
    },
  ];
  if (sourceMode === "microphonePlusSystem") {
    sources.push({
      source: "system",
      state: "starting",
      elapsedMs: 0,
      bytesWritten: 0,
      level: { peak: 0, rms: 0, recentPeaks: [] },
      silenceWarning: false,
      pathFinalized: false,
    });
  }

  return {
    sessionId: "",
    noteId,
    sourceMode,
    state: "starting",
    elapsedMs: 0,
    level: { peak: 0, rms: 0, recentPeaks: [] },
    silenceWarning: false,
    bytesWritten: 0,
    livePreviewEnabled: false,
    sources,
    warnings: [],
  };
}

// Dev-only helper: pass `?fake-recovery=1` in the URL to inject a fake
// recoverable recording so the inline recovery prompt can be iterated
// on without crashing a real recording. No-op in production builds.
export function withFakeRecovery(payload: BootstrapResponse): {
  payload: BootstrapResponse;
  fakeNote?: NoteDto;
} {
  if (!import.meta.env.DEV) return { payload };
  let enabled = false;
  try {
    enabled =
      new URLSearchParams(window.location.search).get("fake-recovery") === "1" ||
      window.location.hash.toLowerCase() === "#fake-recovery" ||
      localStorage.getItem("os-clovy:dev:fake-recovery") === "1";
  } catch {
    return { payload };
  }
  if (!enabled) return { payload };

  const noteId = "fake-recovery-note";
  const sessionId = "fake-recovery-session";
  const now = new Date().toISOString();
  const fakeListItem = {
    id: noteId,
    title: "Team sync",
    preview: "Recovered from an interrupted recording",
    processingStatus: "recoverable" as const,
    folderIds: [],
    createdAt: now,
    updatedAt: now,
  };
  const fakeNote: NoteDto = {
    ...fakeListItem,
    generatedContent: "",
    editedContent: "",
  };
  return {
    payload: {
      ...payload,
      notes: [fakeListItem, ...payload.notes],
      activeRecoveries: [
        {
          sessionId,
          noteId,
          sourceMode: "microphonePlusSystem",
          startedAt: now,
          partialPathPresent: true,
          finalPathPresent: false,
          bytesFound: 2_400_000,
          sources: [
            {
              source: "microphone",
              partialPathPresent: true,
              finalPathPresent: false,
              bytesFound: 1_200_000,
            },
            {
              source: "system",
              partialPathPresent: true,
              finalPathPresent: false,
              bytesFound: 1_200_000,
            },
          ],
        },
        ...payload.activeRecoveries,
      ],
    },
    fakeNote,
  };
}
