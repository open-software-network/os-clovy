import { t } from "../i18n/translate";
/**
 * Orchestration for generating a video from chat.
 *
 * Video generation is async: the native fal adapter returns a job id first, then the
 * caller polls until the desktop bridge has written the mp4 locally. This file
 * stays UI-free and never throws so the chat surface can render running,
 * complete, and error states deterministically.
 */

import { messageFromError } from "./errors";
import type { VideoJobDto, VideoStatusDto } from "./tauri";

export type GenerateChatVideoProgress = Extract<VideoStatusDto, { status: "processing" }> & {
  jobId: string;
};

/** The polling half of the deps: everything needed to follow an already-queued
 * job to completion, without queueing a new one. A resumed job (app relaunch
 * after a crash/reload, or a retry) reuses this so it re-attaches to the same
 * server-side job instead of starting over. */
export type PollChatVideoDeps = {
  pollStatus: (jobId: string) => Promise<VideoStatusDto>;
  onProgress?: (progress: GenerateChatVideoProgress) => void;
  sleep?: (ms: number) => Promise<void>;
  pollIntervalMs?: number;
  maxPolls?: number;
};

export type GenerateChatVideoDeps = PollChatVideoDeps & {
  startGenerate: (
    prompt: string,
    model: string | undefined,
    requestId: string,
    options?: GenerateChatVideoOptions,
  ) => Promise<VideoJobDto>;
  defaultModel?: () => string;
};

export type GenerateChatVideoOptions = {
  duration?: string;
  resolution?: string;
  aspectRatio?: string;
  audio?: boolean;
};

export type GenerateChatVideoResult =
  | {
      status: "ok";
      jobId: string;
      path: string;
      mimeType: string;
      sizeBytes?: number;
      model?: string;
    }
  | {
      status: "error";
      message: string;
      jobId?: string;
      /** True only when the poll budget ran out while the server job was still
       * processing (not a real failure). The job keeps running upstream, so a
       * caller can leave the turn resumable and re-attach on the next launch. */
      stillRunning?: boolean;
    };

const DEFAULT_POLL_INTERVAL_MS = 2_500;
// 360 polls x 2.5s = 900s, matching Clovy's video job timeout. Video jobs
// routinely quote 300-400s+ (queue + render), so
// the earlier 450s budget abandoned jobs the server would have delivered.
const DEFAULT_MAX_POLLS = 360;

export async function generateChatVideo(
  prompt: string,
  deps: GenerateChatVideoDeps,
  model?: string,
  requestId = newVideoRequestId(),
  options: GenerateChatVideoOptions = {},
): Promise<GenerateChatVideoResult> {
  const trimmed = prompt.trim();
  if (!trimmed) {
    return { status: "error", message: t("lib.errors.videoPromptRequired") };
  }

  let job: VideoJobDto;
  try {
    job = await deps.startGenerate(trimmed, model ?? deps.defaultModel?.(), requestId, options);
  } catch (error) {
    return { status: "error", message: messageFromError(error) };
  }

  return pollChatVideo(job.jobId, deps);
}

/**
 * Follows an already-queued video job to completion. Split out of
 * generateChatVideo so a resumed job (relaunch after the app crashed or the dev
 * server hot-reloaded, or a manual retry) polls with the SAME loop and cap
 * instead of a single shot — the job keeps running on the server, so
 * re-attaching recovers the video without a new billable generation.
 */
export async function pollChatVideo(
  jobId: string,
  deps: PollChatVideoDeps,
): Promise<GenerateChatVideoResult> {
  const pollIntervalMs = deps.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS;
  const maxPolls = deps.maxPolls ?? DEFAULT_MAX_POLLS;
  const sleep = deps.sleep ?? defaultSleep;

  for (let attempt = 0; attempt < maxPolls; attempt += 1) {
    let status: VideoStatusDto;
    try {
      status = await deps.pollStatus(jobId);
    } catch (error) {
      return { status: "error", message: messageFromError(error), jobId };
    }

    if (status.status === "completed") {
      return {
        status: "ok",
        jobId,
        path: status.path,
        mimeType: status.mimeType,
        sizeBytes: status.sizeBytes,
        model: status.model,
      };
    }
    if (status.status === "failed") {
      return { status: "error", message: status.reason, jobId };
    }

    deps.onProgress?.({ ...status, jobId });
    if (attempt < maxPolls - 1) {
      await sleep(pollIntervalMs);
    }
  }

  return {
    status: "error",
    message: t("lib.errors.videoStillRunning"),
    jobId,
    stillRunning: true,
  };
}

export function newVideoRequestId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `video-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function defaultSleep(ms: number) {
  return new Promise<void>((resolve) => window.setTimeout(resolve, ms));
}
