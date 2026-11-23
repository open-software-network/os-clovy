// DownloadEvent is owned by the IPC boundary (lib/updater.ts); re-exported here
// so importers do not reach from app/ back up into lib/ for it.
import { t } from "../i18n";
import { messageFromError } from "../lib/errors";
import type { DownloadEvent } from "../lib/updater";
export type { DownloadEvent };

export type UpdatePromptPayload<TUpdate> = {
  update: TUpdate;
  version: string;
  notes?: string;
};

// A release build checks once at launch and whenever the owner asks in
// Settings. A postponed offer stays hidden until one of those runs again.
export type UpdateCheckMode = "launch" | "manual";

// The manual-check success status. Shared so the reporter and the auto-dismiss
// effect that matches on it can't drift apart. It stays an English sentinel in
// state; the update card renders it through t("app.update.upToDate") so it
// follows the interface language.
export const UP_TO_DATE_STATUS = "Clovy is up to date.";

export type UpdateStatusDisplayState = {
  status: string | null;
  leaving: boolean;
  failed: boolean;
};

export type UpdateStatusDisplayAction =
  | { type: "show"; status: string | null; failed?: boolean }
  | { type: "beginUpToDateExit" }
  | { type: "clearUpToDate" };

export const INITIAL_UPDATE_STATUS_DISPLAY: UpdateStatusDisplayState = {
  status: null,
  leaving: false,
  failed: false,
};

// Status changes reset the exit phase synchronously. Timed exit actions are
// guarded by the current value, so a stale success timer cannot fade or clear
// a newer check/failure status if its effect cleanup is delayed.
export function updateStatusDisplayReducer(
  state: UpdateStatusDisplayState,
  action: UpdateStatusDisplayAction,
): UpdateStatusDisplayState {
  if (action.type === "show") {
    const failed = action.status !== null && action.failed === true;
    if (state.status === action.status && state.failed === failed && !state.leaving) return state;
    return { status: action.status, leaving: false, failed };
  }
  if (state.status !== UP_TO_DATE_STATUS) return state;
  if (action.type === "beginUpToDateExit") {
    return { ...state, leaving: true };
  }
  return INITIAL_UPDATE_STATUS_DISPLAY;
}

// The launch check is silent unless it finds an update. Only manual checks
// render the transient checking status and its spinner.
export function updateCheckShowsStatus(mode: UpdateCheckMode): boolean {
  return mode === "manual";
}

export type UpdaterUpdate = {
  version: string;
  body?: string;
  downloadAndInstall: (onEvent?: (event: DownloadEvent) => void) => Promise<void>;
};

export type UpdateCheckDeps<TUpdate extends UpdaterUpdate> = {
  check: () => Promise<TUpdate | null>;
  offer: (payload: UpdatePromptPayload<TUpdate>) => void;
  reportNoUpdate: () => void;
  reportFailure: (message: string) => void;
};

export type PrepareUpdateDeps<TUpdate extends UpdaterUpdate> = {
  update: TUpdate;
  reportProgress: (progress: UpdateInstallProgress) => void;
  reportReady: (payload: UpdatePromptPayload<TUpdate>) => void;
  reportFailure: (message: string) => void;
};

export type UpdateInstallProgress = {
  // waitingForCapture: downloaded, the install waits for a capture to end.
  state: "downloading" | "waitingForCapture" | "installing";
  downloadedBytes?: number;
  contentLength?: number;
};

// A check only offers the update: nothing downloads until the owner accepts.
// A failed launch check (no network, a bad manifest) never interrupts the app.
export async function checkForClovyUpdate<TUpdate extends UpdaterUpdate>(
  deps: UpdateCheckDeps<TUpdate>,
  mode: UpdateCheckMode,
) {
  try {
    const update = await deps.check();
    if (!update) {
      if (mode === "manual") deps.reportNoUpdate();
      return;
    }
    deps.offer({
      update,
      version: update.version,
      notes: normalizeNotes(update.body),
    });
  } catch (error) {
    if (mode === "manual") deps.reportFailure(messageFromUnknown(error));
  }
}

export async function prepareClovyUpdate<TUpdate extends UpdaterUpdate>({
  update,
  reportProgress,
  reportReady,
  reportFailure,
}: PrepareUpdateDeps<TUpdate>) {
  try {
    await downloadAndInstallClovyUpdate(update, reportProgress);
    reportReady({
      update,
      version: update.version,
      notes: normalizeNotes(update.body),
    });
  } catch (error) {
    reportFailure(messageFromUnknown(error));
  }
}

export type UpdateStepState = {
  // A newer version was found and is shown to the owner.
  offered: boolean;
  // The owner chose to update and has not postponed since.
  accepted: boolean;
  // Download and install are running.
  preparing: boolean;
  // The update is installed and only the restart is left.
  ready: boolean;
  relaunching: boolean;
  // A meeting recording or a dictation take is running.
  captureActive: boolean;
};

export type UpdateStep = "none" | "wait" | "install" | "relaunch";

// What the updater does next. Installing on Windows exits the app, and the
// restart on macOS ends it too, so both wait while anything is recording.
export function nextUpdateStep(state: UpdateStepState): UpdateStep {
  if (!state.accepted || state.preparing || state.relaunching) return "none";
  if (!state.ready && !state.offered) return "none";
  if (state.captureActive) return "wait";
  return state.ready ? "relaunch" : "install";
}

async function downloadAndInstallClovyUpdate<TUpdate extends UpdaterUpdate>(
  update: TUpdate,
  reportProgress: (progress: UpdateInstallProgress) => void,
) {
  let downloadedBytes = 0;
  let contentLength: number | undefined;
  // A multi-MB package fires thousands of Progress events; surfacing each as a
  // state update would re-render the app on every network chunk. Throttle
  // downloading ticks to whole-percent changes (or ~1MB steps when the total
  // size is unknown). State transitions (Started and installing) always fire.
  let lastPercent: number | undefined;
  let lastReportedBytes = 0;
  const UNKNOWN_LENGTH_STEP = 1_000_000;

  await update.downloadAndInstall((event) => {
    if (event.event === "Started") {
      downloadedBytes = 0;
      contentLength = event.data.contentLength;
      lastPercent = contentLength ? 0 : undefined;
      lastReportedBytes = 0;
      reportProgress({
        state: "downloading",
        downloadedBytes,
        contentLength,
      });
      return;
    }
    if (event.event === "Progress") {
      downloadedBytes += event.data.chunkLength;
      if (contentLength && contentLength > 0) {
        const percent = Math.round((downloadedBytes / contentLength) * 100);
        if (percent === lastPercent) return;
        lastPercent = percent;
      } else if (downloadedBytes - lastReportedBytes < UNKNOWN_LENGTH_STEP) {
        return;
      } else {
        lastReportedBytes = downloadedBytes;
      }
      reportProgress({
        state: "downloading",
        downloadedBytes,
        contentLength,
      });
      return;
    }
    if (event.event === "WaitingForCapture") {
      reportProgress({ state: "waitingForCapture", downloadedBytes, contentLength });
      return;
    }
    reportProgress({ state: "installing", downloadedBytes, contentLength });
  });
}

function normalizeNotes(notes?: string) {
  const trimmed = notes?.trim();
  return trimmed ? trimmed : undefined;
}

// Native commands reject with an AppError object ({ code, message }), not an Error.
function messageFromUnknown(error: unknown) {
  if (error && typeof error === "object" && "message" in error) return messageFromError(error);
  if (typeof error === "string") return error;
  return t("app.update.unknownError");
}
