import { describe, expect, it, vi } from "vitest";
import {
  generateChatVideo,
  type GenerateChatVideoDeps,
  type PollChatVideoDeps,
  pollChatVideo,
} from "../lib/chat-video-generation";

describe("chat video generation", () => {
  it("starts a job and polls to completion", async () => {
    const deps: GenerateChatVideoDeps = {
      startGenerate: vi.fn().mockResolvedValue({ jobId: "job-1" }),
      pollStatus: vi
        .fn()
        .mockResolvedValueOnce({
          status: "processing",
          averageExecutionMs: 120_000,
          executionMs: 10_000,
        })
        .mockResolvedValueOnce({
          status: "completed",
          path: "/Users/alex/Library/Application Support/dev.guilhermexp.clovy/agent-workspaces/generated-media/videos/out.mp4",
          mimeType: "video/mp4",
          sizeBytes: 1234,
          model: "wan-2.2-a14b-text-to-video",
        }),
      onProgress: vi.fn(),
      sleep: vi.fn().mockResolvedValue(undefined),
    };

    const result = await generateChatVideo(
      "a calm lake",
      deps,
      "wan-2.2-a14b-text-to-video",
      "video-req-1",
    );

    expect(deps.startGenerate).toHaveBeenCalledWith(
      "a calm lake",
      "wan-2.2-a14b-text-to-video",
      "video-req-1",
      {},
    );
    expect(deps.onProgress).toHaveBeenCalledWith({
      jobId: "job-1",
      status: "processing",
      averageExecutionMs: 120_000,
      executionMs: 10_000,
    });
    expect(result).toEqual({
      status: "ok",
      jobId: "job-1",
      path: "/Users/alex/Library/Application Support/dev.guilhermexp.clovy/agent-workspaces/generated-media/videos/out.mp4",
      mimeType: "video/mp4",
      sizeBytes: 1234,
      model: "wan-2.2-a14b-text-to-video",
    });
  });

  it("returns an error for a failed job", async () => {
    const deps: GenerateChatVideoDeps = {
      startGenerate: vi.fn().mockResolvedValue({ jobId: "job-1" }),
      pollStatus: vi.fn().mockResolvedValue({ status: "failed", reason: "content blocked" }),
    };

    await expect(generateChatVideo("a lake", deps)).resolves.toEqual({
      status: "error",
      message: "content blocked",
      jobId: "job-1",
    });
  });

  it("rejects a blank prompt without starting a job", async () => {
    const deps: GenerateChatVideoDeps = {
      startGenerate: vi.fn(),
      pollStatus: vi.fn(),
    };

    await expect(generateChatVideo("   ", deps)).resolves.toEqual({
      status: "error",
      message: "Enter a prompt to generate a video.",
    });
    expect(deps.startGenerate).not.toHaveBeenCalled();
  });

  it("resumes an existing job to completion without re-queueing", async () => {
    // The resume path (app crash / reload / retry): re-attach to a running job
    // by id and poll it to completion — never a single shot, never a new queue.
    const deps: PollChatVideoDeps = {
      pollStatus: vi
        .fn()
        .mockResolvedValueOnce({
          status: "processing",
          averageExecutionMs: 400_000,
          executionMs: 380_000,
        })
        .mockResolvedValueOnce({
          status: "completed",
          path: "/Users/alex/Library/Application Support/dev.guilhermexp.clovy/agent-workspaces/generated-media/videos/out.mp4",
          mimeType: "video/mp4",
          sizeBytes: 42,
          model: "wan-2.2-a14b-text-to-video",
        }),
      onProgress: vi.fn(),
      sleep: vi.fn().mockResolvedValue(undefined),
    };

    const result = await pollChatVideo("job-resumed", deps);

    expect(result).toEqual({
      status: "ok",
      jobId: "job-resumed",
      path: "/Users/alex/Library/Application Support/dev.guilhermexp.clovy/agent-workspaces/generated-media/videos/out.mp4",
      mimeType: "video/mp4",
      sizeBytes: 42,
      model: "wan-2.2-a14b-text-to-video",
    });
    expect(deps.pollStatus).toHaveBeenCalledTimes(2);
  });

  it("flags a still-running job when the poll budget runs out", async () => {
    // The budget-exhausted result must be distinguishable (stillRunning) so a
    // resumer keeps the turn pending and re-attaches on the next launch instead
    // of forcing the user to hit "Try again".
    const deps: PollChatVideoDeps = {
      pollStatus: vi.fn().mockResolvedValue({
        status: "processing",
        averageExecutionMs: 400_000,
        executionMs: 200_000,
      }),
      sleep: vi.fn().mockResolvedValue(undefined),
      maxPolls: 3,
    };

    const result = await pollChatVideo("job-slow", deps);

    expect(result).toEqual({
      status: "error",
      message: "Video generation is still running. Try again later.",
      jobId: "job-slow",
      stillRunning: true,
    });
    expect(deps.pollStatus).toHaveBeenCalledTimes(3);
  });
});
