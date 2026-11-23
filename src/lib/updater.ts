import { Channel, invoke } from "@tauri-apps/api/core";

/**
 * Download progress streamed from the `install_update` command, shaped to match
 * the Rust `DownloadEvent` enum (`{ event, data }`). Defined here, at the IPC
 * boundary that owns the `Channel`, and consumed downward by update-decision.ts.
 */
export type DownloadEvent =
  | { event: "Started"; data: { contentLength?: number } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" }
  // Downloaded; the install waits for a recording or dictation to end.
  | { event: "WaitingForCapture" }
  // The install took its slot: it can no longer be postponed.
  | { event: "Installing" };

/** What `fetch_update` reports: enough to offer, not the live update handle. */
type UpdateMeta = {
  version: string;
  body?: string;
};

/**
 * A frontend-facing update. The real `Update` stays in Rust (it can't be
 * serialized across IPC), so `downloadAndInstall` is a thin bridge to the
 * `install_update` command, shaped to satisfy update-decision.ts's
 * `UpdaterUpdate`.
 */
export type ClovyUpdate = {
  version: string;
  body?: string;
  downloadAndInstall: (onEvent?: (event: DownloadEvent) => void) => Promise<void>;
};

/** Only a release build checks; a development build never does. */
export function updatesEnabled(): Promise<boolean> {
  return invoke<boolean>("updates_enabled");
}

/**
 * Checks the one stable manifest configured in `tauri.conf.json`. The endpoint
 * and the public key never travel through the frontend.
 */
export async function checkClovyUpdate(): Promise<ClovyUpdate | null> {
  const meta = await invoke<UpdateMeta | null>("fetch_update");
  if (!meta) return null;
  return {
    version: meta.version,
    body: meta.body,
    downloadAndInstall: (onEvent) => installStagedUpdate(onEvent),
  };
}

function installStagedUpdate(onEvent?: (event: DownloadEvent) => void): Promise<void> {
  const channel = new Channel<DownloadEvent>();
  if (onEvent) channel.onmessage = onEvent;
  return invoke("install_update", { onEvent: channel });
}

/**
 * Stops an install that waits for a capture to end. The pending
 * `install_update` call then rejects with `update_install_postponed`. Rejects
 * with `update_install_started` once the install has begun.
 */
export function postponeUpdateInstall() {
  return invoke("postpone_update_install");
}

/**
 * Relaunches Clovy to finish an installed update. Routes through the Rust
 * `relaunch_for_update` command instead of a plain process relaunch so Clovy's
 * child processes (the dictation helper and agent harness) are torn down
 * before the process restarts. A bare relaunch can skip that teardown and
 * orphan the helper, which then blocks the new instance from reading
 * permissions.
 */
export function relaunchClovy() {
  return invoke("relaunch_for_update");
}
